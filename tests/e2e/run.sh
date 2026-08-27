#!/usr/bin/env bash
set -euo pipefail

readonly repo_root="${AIOPS_ROOT:-$PWD}"
readonly manifests="$repo_root/tests/e2e/manifests"
readonly cluster_config="$repo_root/tests/e2e/kind.yaml"
readonly namespace="aiops-e2e"
readonly cluster_name="aiops-e2e-$$"
readonly output_dir="$repo_root/target/e2e"

if [[ ! -f "$repo_root/Cargo.toml" || ! -f "$cluster_config" ]]; then
  echo "run this command from the AIOps repository root" >&2
  exit 2
fi

mkdir -p "$output_dir"
test_kubeconfig="$(mktemp)"
export KUBECONFIG="$test_kubeconfig"

cleanup() {
  kind delete cluster --name "$cluster_name" >/dev/null 2>&1 || true
  rm -f "$test_kubeconfig"
}
trap cleanup EXIT

wait_for_json() {
  local description="$1"
  local pod="$2"
  local expression="$3"
  local deadline=$((SECONDS + 120))

  until kubectl -n "$namespace" get pod "$pod" -o json 2>/dev/null |
    jq -e "$expression" >/dev/null 2>&1; do
    if (( SECONDS >= deadline )); then
      echo "timed out waiting for $description" >&2
      kubectl -n "$namespace" get pod "$pod" -o yaml >&2 || true
      return 1
    fi
    sleep 2
  done
}

pod_for_selector() {
  local selector="$1"
  local deadline=$((SECONDS + 120))
  local pod=""

  until [[ -n "$pod" ]]; do
    pod="$(kubectl -n "$namespace" get pods -l "$selector" \
      -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || true)"
    if (( SECONDS >= deadline )); then
      echo "timed out waiting for a pod matching $selector" >&2
      kubectl -n "$namespace" get all >&2 || true
      return 1
    fi
    [[ -n "$pod" ]] || sleep 2
  done

  printf '%s\n' "$pod"
}

diagnose() {
  local pod="$1"
  cargo run --quiet --manifest-path "$repo_root/Cargo.toml" -p aiops-cli -- \
    diagnose pod "$pod" --namespace "$namespace" --lookback 15m \
    >"$output_dir/$pod.json"
}

collect_resource() {
  local kind="$1"
  local name="$2"
  local output="$3"
  cargo run --quiet --manifest-path "$repo_root/Cargo.toml" -p aiops-cli -- \
    collect "$kind" "$name" --namespace "$namespace" --lookback 15m \
    >"$output_dir/$output.json"
}

diagnose_resource() {
  local kind="$1"
  local name="$2"
  local output="$3"
  cargo run --quiet --manifest-path "$repo_root/Cargo.toml" -p aiops-cli -- \
    diagnose "$kind" "$name" --namespace "$namespace" --lookback 15m \
    >"$output_dir/$output.json"
}

has_code() {
  local pod="$1"
  local code="$2"
  jq -e --arg code "$code" \
    'any(.findings[]; .code == $code)' "$output_dir/$pod.json" >/dev/null
}

has_code_prefix() {
  local pod="$1"
  local prefix="$2"
  jq -e --arg prefix "$prefix" \
    'any(.findings[]; .code | startswith($prefix))' "$output_dir/$pod.json" >/dev/null
}

echo "Creating kind cluster $cluster_name"
kind create cluster \
  --name "$cluster_name" \
  --config "$cluster_config" \
  --wait 120s

kubectl apply -f "$manifests/namespace.yaml"
kubectl apply -f "$manifests/healthy.yaml"
kubectl apply -f "$manifests/crash-loop.yaml"
kubectl apply -f "$manifests/image-pull-error.yaml"
kubectl apply -f "$manifests/readiness-failure.yaml"
kubectl apply -f "$manifests/log-patterns.yaml"
kubectl apply -f "$manifests/owned-crash-loop.yaml"

owned_crash_loop_pod="$(pod_for_selector 'app.kubernetes.io/name=owned-crash-loop')"

wait_for_json "healthy pod readiness" healthy \
  'any(.status.conditions[]?; .type == "Ready" and .status == "True")'
wait_for_json "log-pattern pod readiness" log-patterns \
  'any(.status.conditions[]?; .type == "Ready" and .status == "True")'
wait_for_json "failed readiness probe" readiness-failure \
  'any(.status.conditions[]?; .type == "Ready" and .status == "False")'
wait_for_json "CrashLoopBackOff" crash-loop \
  'any(.status.containerStatuses[]?; .restartCount > 0 and .state.waiting.reason == "CrashLoopBackOff")'
wait_for_json "image pull failure" image-pull-error \
  'any(.status.containerStatuses[]?; .state.waiting.reason == "ErrImagePull" or .state.waiting.reason == "ImagePullBackOff")'
wait_for_json "controller-owned CrashLoopBackOff" "$owned_crash_loop_pod" \
  'any(.status.containerStatuses[]?; .restartCount > 0 and .state.waiting.reason == "CrashLoopBackOff")'

for pod in healthy crash-loop image-pull-error readiness-failure log-patterns; do
  diagnose "$pod"
done
diagnose "$owned_crash_loop_pod"

owned_replica_set="$(jq -r '
  .observations.resources[] |
  select(.resource.kind == "replica_set") |
  .resource.name
' "$output_dir/$owned_crash_loop_pod.json" | head -n 1)"

diagnose_resource deployment owned-crash-loop deployment-owned-crash-loop
collect_resource replica-set "$owned_replica_set" replica-set-owned-crash-loop

jq -e '
  (.findings | length == 0) and
  .status == "resolved" and
  .severity == "info" and
  (.observations.errors | length == 0) and
  (.id | type == "string" and length > 0) and
  (.created_at == .updated_at)
' "$output_dir/healthy.json" >/dev/null

has_code_prefix crash-loop container.restart.
has_code crash-loop log.process_crash

has_code image-pull-error pod.not_ready
jq -e 'any(.findings[]; .code | startswith("kubernetes.event."))' \
  "$output_dir/image-pull-error.json" >/dev/null
jq -e '
  .status == "incomplete" and
  any(.observations.errors[];
    .kind == "log_read" and .source == "logs" and .retryable == false)
' "$output_dir/image-pull-error.json" >/dev/null

has_code readiness-failure pod.not_ready
has_code readiness-failure kubernetes.event.unhealthy

has_code log-patterns log.memory_exhausted
has_code log-patterns log.process_crash
has_code log-patterns log.connection_failure
has_code log-patterns log.tls_failure

owned_report="$output_dir/$owned_crash_loop_pod.json"
jq -e --arg pod "$owned_crash_loop_pod" '
  .target.kind == "pod" and
  .target.name == $pod and
  .status == "open" and
  .severity == "critical" and
  (.observations.errors | length == 0) and
  any(.observations.resources[];
    .resource.kind == "deployment" and .resource.name == "owned-crash-loop") and
  any(.observations.resources[];
    .resource.kind == "replica_set") and
  any(.observations.workloads[];
    .resource.kind == "deployment" and .resource.name == "owned-crash-loop") and
  any(.observations.workloads[];
    .resource.kind == "replica_set") and
  any(.observations.relationships[];
    .kind == "controller_owner" and
    .owner.kind == "deployment" and
    .owner.name == "owned-crash-loop" and
    .dependent.kind == "replica_set") and
  any(.observations.relationships[];
    .kind == "controller_owner" and
    .owner.kind == "replica_set" and
    .dependent.kind == "pod" and
    .dependent.name == $pod) and
  any(.findings[];
    .code == "workload.replicas_unavailable" and
    .subject.kind == "deployment" and
    .subject.name == "owned-crash-loop")
' "$owned_report" >/dev/null

has_code_prefix "$owned_crash_loop_pod" container.restart.
has_code "$owned_crash_loop_pod" log.process_crash

jq -e '
  .target.kind == "deployment" and
  .target.name == "owned-crash-loop" and
  .status == "open" and
  any(.observations.relationships[];
    .owner.kind == "deployment" and .dependent.kind == "replica_set") and
  any(.observations.relationships[];
    .owner.kind == "replica_set" and .dependent.kind == "pod") and
  any(.findings[];
    .code == "workload.replicas_unavailable" and
    .subject.kind == "deployment")
' "$output_dir/deployment-owned-crash-loop.json" >/dev/null

jq -e --arg replica_set "$owned_replica_set" '
  .target.kind == "replica_set" and
  .target.name == $replica_set and
  any(.resources[];
    .resource.kind == "deployment" and .resource.name == "owned-crash-loop") and
  any(.relationships[];
    .owner.kind == "deployment" and .dependent.name == $replica_set) and
  any(.relationships[];
    .owner.name == $replica_set and .dependent.kind == "pod") and
  any(.workloads[];
    .resource.kind == "replica_set" and .resource.name == $replica_set)
' "$output_dir/replica-set-owned-crash-loop.json" >/dev/null

echo "All kind end-to-end tests passed. Reports are in $output_dir"
