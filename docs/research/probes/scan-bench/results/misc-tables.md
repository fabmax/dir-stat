| tree | UI | wall ms | batches | max listings per batch |
|---|---|---:|---:|---:|
| `~/.rustup` | normal | 1742 | 35 | 1614 |
| `~/.rustup` | slow200ms | 1467 | 7 | 4307 |
| `~/.rustup` | slow1000ms | 3065 | 3 | 11120 |
| `/System/Library` | normal | 2934 | 59 | 3158 |
| `/System/Library` | slow200ms | 2918 | 14 | 14973 |
| `/System/Library` | slow1000ms | 5077 | 5 | 64190 |

| tree | cancel after | listings received | cancel -> `Finished` | cancel -> all threads joined |
|---|---:|---:|---:|---:|
| `~/dev` | 100 ms | 2323 | 9.7 ms | 9.7 ms |
| `~/dev` | 1000 ms | 23505 | 99.4 ms | 99.5 ms |
| `/System/Library` | 100 ms | 4494 | 8.5 ms | 8.6 ms |
| `/System/Library` | 1000 ms | 59379 | 0.8 ms | 0.8 ms |
| `~/.rustup` | 100 ms | 2440 | 21.1 ms | 21.1 ms |
| `~/.rustup` | 1000 ms | 9114 | 15.9 ms | 16.0 ms |

| one directory with 200,000 files | wall ms (median) | batches | sys ms |
|---|---:|---:|---:|
| custom/std | 515 | 10 | 484 |
| custom/at | 490 | 10 | 476 |
| custom/bulk | 336 | 7 | 335 |
| dua | 377 | - | 360 |

Cancel after 100 ms inside that directory: `Finished` after 2.8 ms.
