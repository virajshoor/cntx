# Apply Mode

Apply mode lets Cntx turn model output into real files while keeping the sandbox
in control.

```bash
cntx --apply --mode allow "add a smoke test for the router"
```

When apply mode is enabled, Cntx asks the model to output complete file contents
as fenced code blocks with a target path:

````text
```rust path=src/example.rs
pub fn example() {}
```
````

Accepted annotations are `path=`, `file=`, and `filename=`. Quoted paths are
accepted too:

````text
```toml path="fixtures/demo config.toml"
name = "cntx"
```
````

Cntx writes only complete blocks with a path annotation. Blocks without a path are
rendered for the user but are not written.

## Checklist

After an apply run, Cntx prints a file checklist:

```text
file checklist
  [written] src/lib.rs write within sandbox
  [outside sandbox] /tmp/other.rs path is outside the sandbox
```

In interactive mode, `/checklist` shows the last apply result until the next apply
run.

## Safety

All apply writes go through the edit sandbox:

- writes inside the project root are allowed or prompted according to the active
  mode
- writes outside the project root are denied unless the root was explicitly added
  with `--allow-write`
- `--dangerously-disable-sandbox` removes path containment and should be reserved
  for workflows that genuinely need it

See [Sandbox](sandbox.md) for the full policy model.
