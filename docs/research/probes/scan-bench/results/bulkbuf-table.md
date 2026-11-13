| tree (bulk, 8 threads) | 8 KiB buffer | 32 KiB buffer | 64 KiB buffer | 256 KiB buffer |
|---|---:|---:|---:|---:|
| `/System/Library` | 2612 ms | 2570 ms | 2515 ms | 2624 ms |
| `~/.rustup` | 1341 ms | 1203 ms | 1276 ms | 1284 ms |
| `/Applications` | 199 ms | 156 ms | 154 ms | 150 ms |

`~/dev`, 8 threads, interleaved: custom/bulk 32 KiB: 18.0, 16.8 s; custom/bulk 256 KiB: 18.8, 17.8 s; custom/std: 22.5, 21.6 s; dua: 15.3, 15.2 s
