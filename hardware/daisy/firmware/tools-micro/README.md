# tools-micro

On-target Daisy helpers: factory-bank QSPI layout and flasher.

| Artifact | Role |
| --- | --- |
| lib | Bank size, CRC32, end-of-flash QSPI address |
| `factory-banks-flash` | Decompress zlib into SDRAM and program QSPI |

```bash
cd hardware/daisy
make factory-banks-flash
```
