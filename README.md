# AIOps

AIOps is a command-line tool that collects information about Kubernetes pods and explains common failures.

It reads pod status, container state, recent events, and container logs. It then produces a JSON report with findings, evidence, and suggested actions.

The project is written in Rust. It is under active development and does not change resources in the cluster.

## Current features

AIOps can:

- Collect pod metadata and health information.
- Collect current and previous container logs.
- Collect recent Kubernetes events.
- Detect pods that are not ready.
- Detect container restarts, crash loops, failed exits, and OOM kills.
- Report Kubernetes warning events.
- Find common memory, crash, connection, and TLS errors in logs.
- Rank and merge related findings.
- Report partial results when some data could not be collected.

## Requirements

The recommended setup uses Nix with flakes enabled.

To use AIOps with a cluster, you also need:

- Access to a Kubernetes cluster through your current kubeconfig.
- Permission to read pods, pod logs, and events.

The end-to-end test also needs Docker or another container runtime supported by kind.

## Getting started with Nix

Download the Rust dependencies:

```console
nix run .#deps
```

Open a development shell with Rust, Cargo, kind, kubectl, and jq:

```console
nix develop
```

Build the program:

```console
nix build
```

The built binary is available at `result/bin/aiops`.

You can also run it without creating a `result` link:

```console
nix run .#run -- --help
```

## Collect pod information

The `collect` command writes a raw observation bundle as JSON:

```console
nix run .#run -- collect pod api-0 --namespace production
```

The default lookback is 15 minutes. The default log limit is 500 lines per container.

You can change the collection settings:

```console
nix run .#run -- collect pod api-0 \
  --namespace production \
  --lookback 30m \
  --tail-lines 200
```

The following flags can disable parts of collection:

- `--no-logs`
- `--no-previous-logs`
- `--no-events`

`--since` is an alias for `--lookback`. Durations can use seconds, minutes, hours, or days, such as `30s`, `15m`, `2h`, or `1d`.

## Diagnose a pod

The `diagnose` command collects the pod information and runs all current diagnostic rules:

```console
nix run .#run -- diagnose pod api-0 --namespace production
```

The result is a JSON diagnosis report. Each finding contains:

- A stable finding code.
- Severity and confidence.
- The affected resource and container.
- A short explanation.
- Supporting evidence.
- Suggested next steps.

If an event or log request fails, the report can still contain useful findings. In that case, `incomplete` is `true` and `collection_errors` explains what failed.

## Run tests

Run formatting checks, Clippy, and all Rust tests:

```console
nix run .#test
```

You can also use Cargo from the development shell:

```console
cargo test --workspace
```

## Run the kind end-to-end test

The end-to-end test creates a temporary local Kubernetes cluster and deploys several test pods:

```console
nix run .#test-kind
```

The test covers:

- A healthy pod.
- A crash-looping pod.
- An image pull failure.
- A failed readiness probe.
- Known error patterns in logs.

The script waits for each expected Kubernetes state, runs AIOps, and checks the JSON findings. Reports are written to `target/e2e/`.

The test uses a separate temporary kubeconfig and a unique cluster name. It deletes the kind cluster when the test finishes or fails.

## Nix commands

| Command | Purpose |
| --- | --- |
| `nix run .#run -- <args>` | Run AIOps |
| `nix run .#deps` | Download locked Cargo dependencies |
| `nix run .#test` | Run formatting, lint, and unit tests |
| `nix run .#test-kind` | Run the kind end-to-end test |
| `nix develop` | Open the development shell |
| `nix build` | Build the release package |

## Safety

AIOps currently performs read-only Kubernetes operations. The kind test creates resources only inside its temporary local cluster.
