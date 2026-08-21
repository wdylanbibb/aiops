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

diagnose() {
  local pod="$1"
  cargo run --quiet --manifest-path "$repo_root/Cargo.toml" -p aiops-cli -- \
    diagnose pod "$pod" --namespace "$namespace" --lookback 15m \
    >"$output_dir/$pod.json"
}

has_code() {
  local pod="$1"
  local code="$2"
  jq -e --arg code "$code" \
    'any(.findings[]; .code == $code)' "$output_dir/$pod.json" >/dev/null
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

for pod in healthy crash-loop image-pull-error readiness-failure log-patterns; do
  diagnose "$pod"
done

jq -e '.findings | length == 0' "$output_dir/healthy.json" >/dev/null

has_code crash-loop container.restart.crash_loop
has_code crash-loop log.process_crash

has_code image-pull-error pod.not_ready
jq -e 'any(.findings[]; .code | startswith("kubernetes.event."))' \
  "$output_dir/image-pull-error.json" >/dev/null

has_code readiness-failure pod.not_ready
has_code readiness-failure kubernetes.event.unhealthy

has_code log-patterns log.memory_exhausted
has_code log-patterns log.process_crash
has_code log-patterns log.connection_failure
has_code log-patterns log.tls_failure

echo "All kind end-to-end tests passed. Reports are in $output_dir"
