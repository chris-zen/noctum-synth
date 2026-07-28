# tools-host

Host-side helpers under the firmware tree. Build for the host triple (the Daisy
workspace defaults to `thumbv7em-none-eabihf`).

```bash
HOST=$(rustc -vV | sed -n 's/^host: //p')
cargo run --release -p tools-host --target "$HOST" --bin <name> -- [args...]
```

## Tools

| Binary | Purpose |
| --- | --- |
| `factory-banks-compress` | Validate and zlib-compress the Rev2 factory `.syx` |

Depends on [`tools-micro`](../tools-micro/) for layout/CRC. Invoked
automatically by `make factory-banks-flash`.
