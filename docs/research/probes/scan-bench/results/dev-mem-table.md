| configuration (`~/dev`, bulk, 8 threads) | files kept | tree nodes | peak RSS MiB | wall s |
|---|---:|---:|---:|---:|
| UI tree + every file kept individually (`--threshold 0`) | 3,069,871 | 533,523 | 405 | 16.5 |
| UI tree + files >= 1 MiB (default) | 31,458 | 533,523 | 162 | 17.2 |
| UI tree, no individual files (`--threshold max`) | 0 | 533,523 | 160 | 15.1 |
| scanner only, no UI tree (`--no-model`) | 0 | 0 | 21 | 16.5 |
| default again, under `/usr/bin/time -l` | 31,458 | 533,523 | 164 | 16.4 |
