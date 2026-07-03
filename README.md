# driftguard

**Post-IaC server verification.** After Terraform provisions it and Ansible configures it, driftguard checks that the box actually looks the way your code says it should — then fails your pipeline if it doesn't.

```
  terraform apply / ansible-playbook site.yml
                     |
                     v
  driftguard run checks.yaml --host admin@$(terraform output -raw ip)
                     |
            exit 0 = deployed as intended
            exit 1 = drift / broken deployment
```

driftguard is a single static binary (Rust), configured with plain YAML, that runs checks locally or over SSH and reports in terminal, JSON, or JUnit format — so your CI can treat deployment verification like a test suite.

## Install

```bash
cargo install --git https://github.com/kelleyblackmore/driftguard
# or clone and build
git clone https://github.com/kelleyblackmore/driftguard && cd driftguard && cargo build --release
```

## Quick start

```bash
driftguard init                 # writes an example driftguard.yaml
driftguard validate driftguard.yaml
driftguard run driftguard.yaml                          # check this machine
driftguard run driftguard.yaml --host admin@10.0.0.5    # check a remote host over SSH
```

## Check types

| type | verifies | key parameters |
|------|----------|----------------|
| `file` / `directory` / `symlink` | existence, content, permissions | `path`, `exists`, `contains`, `content_pattern`, `permissions` |
| `command` | exit status and output | `command`, `exit_status`, `stdout: {contains, pattern}`, `stderr: {...}` |
| `service` | running / enabled (systemd, SysV) | `service`, `running`, `enabled` |
| `process` | process presence and instance count | `process`, `running`, `count`, `min_count`, `max_count` |
| `port` | TCP/UDP listener present | `port`, `protocol`, `listening` |
| `package` | installed via dpkg/rpm/apk/pacman | `package`, `installed`, `version` |
| `config` | keys/values in JSON or YAML files | `path`, `format`, `has_key`, `has_value` |

## Configuration

```yaml
title: Web server deployment checks

variables:
  WEB_ROOT: /var/www/html

checks:
  - name: nginx running and enabled
    type: service
    service: nginx
    running: true
    enabled: true

  - name: web root exists
    type: directory
    path: "{{ WEB_ROOT }}"

  - name: listening on 80
    type: port
    port: 80

  - name: app config rendered correctly
    type: config
    path: /etc/myapp/config.json
    has_value:
      service.port: 8080
```

`{{ VAR }}` references are substituted from the `variables:` section and from repeatable `--var KEY=VALUE` CLI flags (CLI wins). That's the bridge from your IaC outputs:

```bash
driftguard run checks.yaml \
  --host "admin@$(terraform output -raw public_ip)" \
  --var "APP_PORT=$(terraform output -raw app_port)"
```

## CI integration

driftguard exits `0` when everything passes, `1` when any check fails, and `2` on configuration or execution errors. JUnit output plugs into CI test reporting:

```yaml
# GitHub Actions — after your terraform/ansible steps
- name: Verify deployment
  run: driftguard run checks.yaml --host "deploy@${{ steps.tf.outputs.ip }}" -o junit -f results.xml

- uses: dorny/test-reporter@v1
  if: always()
  with:
    name: deployment checks
    path: results.xml
    reporter: java-junit
```

### With Ansible

Keep `checks.yaml` next to your playbook and run driftguard as the last play step (via `command:` on the control node) or as a separate pipeline stage:

```bash
ansible-playbook -i inventory site.yml
driftguard run checks.yaml --host "$(ansible-inventory --host web1 | jq -r .ansible_host)"
```

## Remote execution

Remote targets use the system OpenSSH client (`ssh` must be on PATH — standard on Linux, macOS, and Windows 10+). driftguard runs with `BatchMode=yes`, so key-based auth must be configured; it will fail fast rather than hang on a password prompt. `~/.ssh/config` aliases work as targets.

Freshly provisioned hosts present host keys your CI runner has never seen. Pass `--accept-new-host-key` (OpenSSH `StrictHostKeyChecking=accept-new`) to trust a host's key on first contact while still rejecting *changed* keys:

```bash
driftguard run checks.yaml --host "deploy@$NEW_VM_IP" --accept-new-host-key
```

## How it's tested

CI runs driftguard against itself three ways: the unit suite (mock-runner tests for every check type), the local integration job (checks against the CI runner), and a **container integration job** that builds an ubuntu+sshd+nginx target container, runs the full check suite against it *over SSH*, and then deliberately breaks the deployment to assert driftguard exits `1` on drift. See `tests/container/`.

Quality gates on every push:

- **Coverage** — `cargo llvm-cov` with a 70% line floor (currently ~76%); lcov report uploaded as an artifact
- **Supply chain** — `cargo audit` against the RustSec advisory database (also weekly on a schedule), CycloneDX SBOM (JSON + XML) generated and uploaded as an artifact
- **Dependabot** — weekly cargo and github-actions update PRs, minor/patch grouped

## Compatibility

driftguard reads [serverinspector](https://github.com/kelleyblackmore/ServerInspector)-style configs: the `tests:` alias and `environment.variables:` section are both accepted, so existing suites port over mostly unchanged.

## Platform notes

Check targets are primarily Linux servers (the post-IaC use case). Running driftguard itself works anywhere. Local Windows targets support `file`, `command`, `port`, `process`, and `config` checks; `service` supports running-state via `sc query`; `package` checks are Unix-only.

## License

MIT
