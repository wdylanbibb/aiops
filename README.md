# AIOps

AIOps is a read-only Kubernetes diagnostics CLI written in Rust. It collects workload state,
ownership relationships, pod health, events, and container logs, then produces a structured
Incident with ranked, evidence-backed findings and recommended next steps.

The current release is a completed MVP. It uses deterministic diagnostic rules and does not call
an external AI service or modify resources in the target cluster.

## Highlights

- Diagnoses Pods, ReplicaSets, and Deployments.
- Resolves the Deployment → ReplicaSet → Pod controller-owner graph.
- Collects resource state, workload replica status, pod and container health, recent events, and
  current or previous container logs.
- Detects rollout failures, unavailable replicas, unready pods, container restarts, warning events,
  and common failure signatures in logs.
- Returns useful findings when individual evidence requests fail and marks the Incident
  `incomplete`.
- Produces deterministic JSON suitable for scripts and downstream services.
- Includes Rust unit tests and a disposable kind end-to-end suite.

## Project layout

| Crate | Responsibility |
| --- | --- |
| `aiops-core` | Stable resource, observation, finding, evidence, and Incident contracts |
| `aiops-k8s` | Kubernetes clients, topology discovery, evidence collection, and conversion |
| `aiops-diagnostics` | Deterministic diagnostic rules, deduplication, and finding ranking |
| `aiops-cli` | `aiops` command-line interface and JSON output |

## Requirements

- Linux on `x86_64` or `aarch64` for the supported Nix flake outputs.
- Nix with flakes enabled, or Rust 1.85 or newer with Cargo.
- A current kubeconfig and read access to Pods, Pod logs, Events, ReplicaSets, and Deployments.
- Docker or another kind-compatible container runtime only when running the end-to-end suite.

## Quick start

Open the development environment and build the binary:

```console
nix develop
nix build
./result/bin/aiops --help
```

You can also run directly through the flake:

```console
nix run .#run -- diagnose deployment api --namespace production
```

Without Nix:

```console
cargo build --release -p aiops-cli
./target/release/aiops --help
```

## Commands

`collect` writes a raw `ObservationBundle`:

```console
aiops collect pod api-0 --namespace production
aiops collect replica-set api-7b9f6d8c5 --namespace production
aiops collect deployment api --namespace production
```

`diagnose` collects the same evidence, evaluates all rules, and writes an `Incident`:

```console
aiops diagnose pod api-0 --namespace production
aiops diagnose replica-set api-7b9f6d8c5 --namespace production
aiops diagnose deployment api --namespace production
```

`replicaset` and `rs` are aliases for `replica-set`; `deploy` is an alias for `deployment`.

Every resource command supports:

| Option | Default | Purpose |
| --- | --- | --- |
| `-n, --namespace` | `default` | Namespace containing the target |
| `--lookback`, `--since` | `15m` | Event and log collection window |
| `--tail-lines` | `500` | Maximum log lines per container request |
| `--no-logs` | off | Disable current container logs |
| `--no-previous-logs` | off | Disable logs from restarted containers |
| `--no-events` | off | Disable Kubernetes event collection |

Durations accept `s`, `m`, `h`, or `d`, such as `30s`, `15m`, `2h`, or `1d`.

Pod targets collect logs for all containers. ReplicaSet and Deployment targets collect logs only
from owned Pods whose health is not `healthy`; this keeps workload-level output bounded and focused.

## Understanding the output

An Incident has one of three statuses:

- `resolved`: collection completed and no warning or critical findings were produced.
- `open`: collection completed and at least one warning or critical finding was produced.
- `incomplete`: at least one evidence request or topology lookup failed. Findings may still be
  present and useful.

Incident severity is the highest finding severity, or `info` when there are no findings. Each
finding includes a stable code, severity, confidence, subject, explanation, supporting evidence,
and recommendations.

```console
aiops diagnose deployment api -n production > incident.json
jq '{id, status, severity, findings: [.findings[].code]}' incident.json
```

Collection errors are preserved in `observations.errors` with a source, structured kind, message,
and retryability indicator. An incomplete Incident is not the same as an unsuccessful command: the
CLI exits successfully when the target was collected but some optional evidence was unavailable.

## Kubernetes permissions

AIOps performs only `get` and `list` requests. A namespaced Role suitable for diagnosis is:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: aiops-reader
  namespace: production
rules:
  - apiGroups: [""]
    resources: ["pods", "pods/log", "events"]
    verbs: ["get", "list"]
  - apiGroups: ["apps"]
    resources: ["deployments", "replicasets"]
    verbs: ["get", "list"]
```

Bind the Role to the user or service account represented by the active kubeconfig. Repeat it in
each namespace that AIOps should inspect.

## Development and testing

```console
nix run .#test
nix run .#test-kind
```

The first command checks formatting, runs Clippy with warnings denied, and executes all workspace
tests. The kind suite creates an isolated cluster, exercises real Pod and workload failures, checks
the resulting JSON contracts, and deletes the cluster on exit.

## Limitations and safety

- AIOps does not watch resources continuously, persist Incidents, or correlate separate runs.
- It does not collect metrics or traces.
- It supports Pod, ReplicaSet, and Deployment targets; other Kubernetes workload kinds are modeled
  but not collected.
- Findings are deterministic operational signals, not proof of root cause.
- Logs and Events may contain sensitive application data. Treat exported JSON accordingly.
- AIOps never patches, deletes, restarts, scales, or otherwise mutates target resources.

## Future direction

The next planned phase is a separate local service that translates Incidents into a structured,
evidence-referenced explanation through a local vLLM endpoint. It is intentionally not part of the
current MVP.

## Nix commands

| Command | Purpose |
| --- | --- |
| `nix run .#run -- <args>` | Run `aiops` |
| `nix run .#deps` | Download locked Cargo dependencies |
| `nix run .#test` | Run formatting, lint, and unit tests |
| `nix run .#test-kind` | Run the kind end-to-end suite |
| `nix develop` | Open the development shell |
| `nix build` | Build the release package |
