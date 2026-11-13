# dir-stat: filesystem-semantics checklist

What a disk-usage analyzer has to get right so that its **numbers are correct** and its **scan is safe**.
Platform priority: macOS (APFS) > Linux > Windows.

**Evidence base.** Most claims were measured on the target machine: macOS 26.7.1 (25G241), Apple Silicon, internal 1 TB APFS container, FileVault on. The terminal session doing the probing has **no** Full Disk Access.
- Probe source: `/tmp/dirstat-work/probes/fs-probe/src/main.rs` (Rust, `libc 0.2.191`). Run `target/release/fs-probe <cmd>` with one of `mounts|statfs|volspace|volcaps|attrs|bulk|iopol|opendir|walk|bwalk|names|longpath`.
- Raw outputs: `/tmp/dirstat-work/probes/fs-probe/evidence/01..07-*.txt`.
- Reference sources read (read-only):
  - xnu `bsd/vfs` (xnu-12377.1.9)
  - Apple `file_cmds` (du/df)
  - ncdu 2.9.2
  - gdu (2026-10)
  - dua-cli (2026-11-02)
  - `trash` 5.2.9
  - `filesize` 0.2.0
  - Rust std 1.93 sources
  - the MacOSX27 SDK headers

Per item, the fields are:
- **Issue**: the problem.
- **Evidence**: measurement on this machine, or a cited source.
- **v1**: recommended handling for the first version.
- **UI**: what to show the user.
- **Later**: a v2 idea, where there is one.

Legend for "verified": (M) measured here, (S) read in source or SDK, (R) reported by third parties and not verified here.

---

## 0. The rules in one screen

1. **Primary size is allocated size** ("size on disk") = `st_blocks * 512`. This is exactly what `du` reports (M: identical KiB on `/opt/homebrew` and `~/.rustup`). Apparent size (`st_size`) is a secondary column.
2. **Never follow symlinks.** Use `lstat`/`fstatat(AT_SYMLINK_NOFOLLOW)`; `DirEntry::metadata()` already does this. Count the link itself.
3. **Count hard links once** per scan, keyed by `(st_dev, st_ino)` and only when `st_nlink > 1`. The key is a concurrent set whose `insert()` returns a bool.
4. **Boundaries come from a mount-table snapshot taken at scan start, not only from `st_dev`.** On macOS the System and Data volumes share one `st_dev` (M). A `-x`-style dev check therefore walks into `/System/Volumes/Data` and counts about 816 GB twice.
5. **Skip every mount point except the scan root's own volume**, before calling `stat` on it. This avoids hangs on network and autofs mounts. On macOS, the Data-volume remainder under `/System/Volumes/Data` gets special treatment when the root is `/`; see §10.
6. **macOS process and thread policy before scanning:**
   - dataless materialization OFF, and never `readdir` a directory flagged `SF_DATALESS`;
   - autofs trigger resolution OFF;
   - atime updates OFF;
   - raise `RLIMIT_NOFILE`, because launchd's soft limit is **256** (M).
7. **Classify errors:**
   - `EACCES` means Unix permissions; Full Disk Access (FDA) will not help.
   - `EPERM` means macOS privacy, SIP or sandbox; FDA often helps.
   - `EDEADLK` means cloud content that is not downloaded.
   - `ENOENT`/`ENOTDIR` means the item vanished; drop it silently.
   - `ENAMETOOLONG` means the path is too long.
8. **Names stay `OsString`/`PathBuf` end to end**; display them with `to_string_lossy()`. Never normalize names or change their case.
9. **Volume used space on APFS comes from `getattrlist(ATTR_VOL_SPACEUSED)`, not `statfs`.** On APFS, `statfs` reports container-wide numbers (M).
10. **Ask for Full Disk Access up front** before scanning `~` or `/`. The deep link is `x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles` (S, M).
11. **Delete = move to Trash via `NSFileManager`**: `trash` crate with `DeleteMethod::NsFileManager`. Apply hard guards (§9) and re-validate `(dev, ino)` before acting.
12. **Present numbers honestly.** Clones, snapshots, purgeable space and the Trash mean "deleting X frees ≤ X". Say so in the UI.

---

## 1. Sizes

### 1.1 Apparent vs allocated size
- **Issue:**
  - `st_size` is the logical length.
  - `st_blocks*512` is what the filesystem allocated. That is what deleting frees, modulo sharing (§1.4).
  - The two diverge for sparse files, compressed files, dataless files and small files.
- **Evidence (M):**
  - The `walk` probe and `du -skx` give **identical totals**:
    - `/opt/homebrew`: 6,193,952 KiB.
    - `~/.rustup`: 10,639,920 KiB.
  - Apparent size is ~6 % smaller there (block rounding).
  - `/usr`: apparent 1.58 GB vs allocated 0.79 GB, because of compression.
- **v1:**
  - Aggregate both per node.
  - Sort and size by allocated.
  - On Windows v1, allocated = logical length; see §4.
- **UI:**
  - Column "Size on disk" (default sort).
  - Secondary column or tooltip "Size" (apparent).
  - Tooltip: "Space allocated on disk (same as `du`). Deleting may free less if data is shared with clones or snapshots."

### 1.2 Sparse files
- **Issue:** VM disks, Docker's `Docker.raw`, databases and `mkfile -n` output have huge `st_size` but few allocated blocks.
- **Evidence (M):**
  - `sparse1g.bin`: size 1,073,741,824, `st_blocks`=32 (16 KiB).
  - `ATTR_CMNEXT_EXT_FLAGS` reports `EF_IS_SPARSE`.
  - `du -skA` on the scratch dir is 1,146,338 KiB vs `du -sk` 88,020 KiB.
- **v1:** Allocated size only. Mark a file sparse when `alloc + 4096 < size` and it is not dataless (the heuristic the probe uses).
- **UI:** Small "sparse" badge and tooltip "Logical size 1.0 GB, uses 16 KB on disk".

### 1.3 APFS transparent compression (decmpfs)
- **Issue:**
  - System files and `ditto --hfsCompression` output carry `UF_COMPRESSED`.
  - Data lives in the `com.apple.decmpfs` xattr or the resource fork.
- **Evidence (M):**
  - `st_blocks` already reflects the compressed allocation:
    - `/usr/bin/zip` 404,864 B vs 184,320 B on disk;
    - a 10 MB text file compresses to 86,016 B.
  - `ATTR_FILE_ALLOCSIZE` == `st_blocks*512` for compressed files.
  - Compressed-file counts:
    - `/usr`: 13,743 of 18,168 files;
    - `/System/Library`: 108,411 files, 11.37 GB apparent → 5.52 GB allocated.
  - **Pitfall:** `ATTR_CMNEXT_PRIVATESIZE` returns **0** for compressed files, so a private-size sum undercounts them.
- **v1:** Nothing special (`st_blocks` is right). Optionally read `st_flags & UF_COMPRESSED` via `std::os::macos::fs::MetadataExt::st_flags()`.
- **UI:** Optional "compressed" hint in the details pane ("10.0 MB → 86 KB on disk").

### 1.4 APFS clones (shared extents)
- **Issue:**
  - `cp -c`, Finder duplicate, Xcode and some installers create clones that share blocks.
  - `st_blocks` counts the full allocation for **every** clone, so `du`, Finder and we overcount.
  - Deleting one clone frees only its private blocks.
- **Evidence (M, scratch dir):**

  | file | st_blocks*512 | PRIVATESIZE | EXT_FLAGS | CLONEID | CLONE_REFCNT |
  |---|---|---|---|---|---|
  | `random20m.bin` (original) | 20,000,768 | 0 | MAY_SHARE_BLOCKS \| SHARES_ALL_BLOCKS | 261065156 | 2 |
  | `clone20m.bin` (`cp -c`) | 20,000,768 | 0 | MAY_SHARE_BLOCKS \| SHARES_ALL_BLOCKS | 261065156 | 2 |
  | `clone2_20m.bin` (clone, then 2 MiB overwritten) | 20,000,768 | 2,097,152 | MAY_SHARE_BLOCKS | 261065160 | 1 |
  | `copy20m.bin` (real copy) | 20,000,768 | 20,000,768 | – | own | 1 |

  - `du -sk` counts all four fully.
  - `/Applications` contains 1,138 files (1.03 GB) with `EF_SHARES_ALL_BLOCKS` and `CLONE_REFCNT` ≥ 2 whose partners live **outside** `/Applications`. Deleting those apps frees roughly nothing for those files.
  - Files on the sealed system volume report PRIVATESIZE 0 because they live in a snapshot.
- **Detection cost (M):** the `bwalk` probe over `~/.rustup` (454k entries), single thread, warm cache, machine load ≈ 6.5, so timings are noisy:
  - `getattrlistbulk` without extended attributes: 3.5–7.1 s.
  - Plus `ATTR_CMNEXT_EXT_FLAGS|CLONEID|CLONE_REFCNT`: 6.7–7.9 s.
  - Plus `ATTR_CMNEXT_PRIVATESIZE`: 10.4–11.2 s.
  - `std::fs` (`readdir` + `fstatat`): 8.0–8.9 s.
  - Conclusion: clone *flags* cost about the same as a plain `std` walk. `PRIVATESIZE` is the expensive one.
- **APIs (S):**
  - `getattrlist`/`getattrlistbulk` with `FSOPT_ATTR_CMN_EXTENDED`. The attrs go in `forkattr`:
    - `ATTR_CMNEXT_PRIVATESIZE`: "bytes ... not trapped inside a clone or snapshot, and which would be freed immediately if the file were deleted";
    - `ATTR_CMNEXT_CLONEID`;
    - `ATTR_CMNEXT_EXT_FLAGS`: `EF_MAY_SHARE_BLOCKS`=0x1, `EF_SHARES_ALL_BLOCKS`=0x40, `EF_IS_PURGEABLE`=0x8, `EF_IS_SPARSE`=0x10, `EF_IS_SYNC_ROOT`=0x4;
    - `ATTR_CMNEXT_CLONE_REFCNT`=0x1000 ("number of full clones").
  - Volume capability check: `VOL_CAP_FMT_CLONE_MAPPING`. All CMNEXT attrs are valid on this machine's APFS volumes.
  - Foundation equivalents: `NSURLMayShareFileContentKey`, `NSURLFileContentIdentifierKey`.
- **v1:**
  - Count allocated size per file, the same as `du` and Finder. Do **not** try to dedupe.
  - Wording must make "may free less" explicit.
- **Later (macOS backend):**
  - A `getattrlistbulk` walker requesting `EXT_FLAGS|CLONEID`. Show a "clone" badge for `EF_MAY_SHARE_BLOCKS`.
  - Optional setting "count identical clones once", deduping `EF_SHARES_ALL_BLOCKS` files by `(dev, CLONEID)`. dua-cli ships exactly this as `--deduplicate-apfs-clones` (S).
  - Per-selection, on-demand "Freed immediately if deleted ≈ Σ" computed as:
    - `PRIVATESIZE` for files with `EF_MAY_SHARE_BLOCKS`;
    - allocated size otherwise, which works around compressed files reporting 0;
    - plus one copy per clone group whose `CLONE_REFCNT` members all lie inside the selection.
- **UI:**
  - Footnote on totals: "APFS clones and snapshots can share space; totals can exceed the disk's used space."
  - Later: badge "Clone – shares data with N other file(s)".

### 1.5 Extended attributes and resource forks
- **Issue:** Large xattrs (stored as streams on APFS) take space but are not in `st_blocks`.
- **Evidence (M):** A 1-byte file with a 200 KB xattr has `st_blocks`=8 and `ATTR_FILE_ALLOCSIZE`=4096.
- **v1:** Ignore. It is a small systematic undercount, and `du` ignores it too.
- **UI:** none.

### 1.6 Directories' own blocks
- **Issue:** `du` counts each directory's own allocation.
- **Evidence (M):**
  - On APFS every directory has `st_blocks`=0, even one with 3,000 entries (`ATTR_DIR_ALLOCSIZE`=0); entries live in the filesystem B-tree.
  - On ext4 and FAT, directories use 4 KiB+ (not measured here).
- **v1:** Add `st_blocks*512` of every directory, including the root, to itself. This gives du parity everywhere at zero cost.
- **UI:** none.

### 1.7 Symlinks and small files
- **Evidence (M):**
  - APFS symlinks: `st_blocks`=0 even with a 600-byte target (stored inline).
  - Files: 1 byte → 4,096; 4,097 bytes → 8,192 (block size 4 KiB).
  - Empty files → 0.
- **v1:** Count `lstat` blocks of the link itself. **Never** resolve it.
- **UI:** Symlink rows show "→ target" (from `read_link`) and are not expandable.

### 1.8 Hard links
- **Issue:** One inode reachable under several names. Counting each link inflates totals.
- **Evidence (M):**
  - `/usr`: 4,638 hard-linked files, 2,903 duplicate links = 26 MB.
  - `/System/Library`: 28,234 hard-linked files, 22,447 duplicates = **820 MB**, about 5 % of its 16.6 GB.
  - Unique hard-linked inodes needed in the set: 1,735 (`/usr`), 5,787 (`/System/Library`).
  - APFS volume group: System and Data share `st_dev` 16777232, but inode spaces are disjoint. System inodes look like `0x0FFFFFFF_xxxxxxxx`; Data inodes are small numbers. So `(st_dev, st_ino)` stays a valid key.
- **How others do it:**
  - Apple `du` and gdu: count at first encounter (gdu keys by inode only, which can collide across devices).
  - dua: `(dev, ino)` → remaining links; drop the entry when it reaches 0, which bounds memory (S).
  - ncdu 2: computes per directory a "unique" size (each inode once) and a "shared" size (links also outside this dir = space not reclaimable). This is order-independent, done after the scan (S).
  - Directory hard links (old HFS+ Time Machine) and macOS firmlinks are listed in ncdu's BUGS as "scanned and counted multiple times".
- **v1:**
  - Track only `nlink > 1` non-directories in a sharded concurrent set (`DashSet<(u64,u64)>` or `[Mutex<HashSet>; 64]`).
  - `insert()` returns `true` for exactly one thread. That thread counts the size, which makes it race-free.
  - Memory is ~32–48 B per entry: a few MB for a whole macOS system; Linux ostree/flatpak hard-link farms could reach tens of MB.
  - Attribution (which directory gets the bytes) is nondeterministic in a parallel walk. Totals above the common ancestor are deterministic.
- **UI:**
  - Badge "H" (hard link: counted at another path) and size shown greyed.
  - Details: "N links; this data is only freed when all links are deleted."
- **Later:** ncdu-style unique/shared columns.

### 1.9 Purgeable space
- **Issue:**
  - macOS counts some space as "available" that `statfs` does not: caches, evictable iCloud files and snapshots via CacheDelete.
  - Finder shows the larger number.
- **Evidence (M, `/`):**
  - `NSURLVolumeAvailableCapacityKey` = 140.72 GB, which equals `statfs` `f_bavail*f_bsize`.
  - `NSURLVolumeAvailableCapacityForImportantUsageKey` = 144.08 GB, so purgeable ≈ **3.4 GB**.
  - `...ForOpportunisticUsageKey` = 134.04 GB.
  - Per-file flag `EF_IS_PURGEABLE` / `NSURLIsPurgeableKey` exists; no purgeable files were found in the scanned trees.
- **v1:** Show free space = `statfs` `f_bavail*f_bsize`. If the objc2-foundation dependency is acceptable, also show "+ ~3.4 GB purgeable" from the Important-usage key. Otherwise skip that.
- **UI:**
  - "142.8 GB available (macOS can free ~3.4 GB more on demand)".
  - Never present purgeable as something the user should delete.

### 1.10 Local snapshots (Time Machine and OS update)
- **Issue:** Blocks referenced by an APFS snapshot are not freed when the live file is deleted. Snapshot space is invisible to any directory scan.
- **Evidence (M):**
  - `tmutil listlocalsnapshots /` and `diskutil apfs listSnapshots disk3s3` show 3 `com.apple.os.update-*` snapshots on the System volume.
  - The Data volume shows "No snapshots" and no Time Machine destination is configured.
  - System-volume files report `PRIVATESIZE` 0 because their data is trapped in the sealed snapshot.
- **v1:** Nothing during the scan. After a delete, if `tmutil listlocalsnapshots /` (spawned, cheap) lists any `com.apple.TimeMachine.*` snapshot, show the hint below.
- **UI:** "Local Time Machine snapshots exist; freed space may appear only after macOS removes them (automatically within ~24 h or when space is needed)."

### 1.11 iCloud Drive / File Provider dataless files and directories
- **Issue:**
  - With "Optimize Mac Storage", iCloud Drive and File Provider clouds (Dropbox, OneDrive, Google Drive under `~/Library/CloudStorage`) evict content.
  - Evicted **files** are "dataless": `SF_DATALESS` (0x40000000) is set, `st_size` is the full logical size, and `st_blocks` ≈ 0.
  - **Directories** can be dataless too: their listing has not been fetched yet.
- **What triggers downloads:**
  - `lstat` does **not** (metadata is local).
  - **Reading a dataless directory or opening a dataless file does.** The kernel upcalls the resolver (S: xnu `vfs_materialize_dir`/`vfs_materialize_file`). Reading blocks until the download finishes: devclean saw an 8-minute stall in one `readdir` under `~/Library/Mobile Documents` (R).
- **Evidence (M, S):**
  - This process starts with `getiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_PROCESS)` = **2 (ON)**. Normal processes therefore **do** materialize.
  - The kernel returns `EDEADLK` when materialization is prevented (xnu `vfs_context_dataless_materialization_is_prevented`).
  - `setiopolicy_np(..., IOPOL_MATERIALIZE_DATALESS_FILES_OFF)` succeeds. Afterwards `sysctl vfs.nspace.prevent_materialization` reads 1, which is the same knob Apple's `du` sets ("du should not have any side effect on disk usage, so prevent materializing dataless directories upon traversal", file_cmds `du.c`).
  - Per-thread decorations override the process policy; Foundation uses them.
  - devclean (R) measured that even with the OFF policy, a dataless directory read still downloaded (17 s). So also **check the flag before descending**.
  - No iCloud Drive exists on this machine (`~/Library/Mobile Documents` absent). Not tested live, deliberately.
- **v1:**
  1. On each scanner thread call `setiopolicy_np(3 /*MATERIALIZE_DATALESS*/, 1 /*THREAD*/, 1 /*OFF*/)`. Verified: the thread policy reads back 1 (M, `evidence/04b`). Process scope (0) also works but is inherited by spawned children.
  2. Never `readdir`/`getattrlistbulk` a directory whose `lstat` `st_flags` has `SF_DATALESS`; make it a leaf.
  3. Never open files.
  4. Map `EDEADLK` (errno 11) on any directory operation to the CloudNotDownloaded state.
  5. Count dataless files with their (≈0) allocation.
- **Later:** In a bulk walker, `EF_IS_SYNC_ROOT` cheaply flags File Provider sync roots.
- **UI:**
  - Cloud badge. "Not downloaded – uses 0 B on this Mac (5.2 GB in iCloud)".
  - For a dataless folder: "Cloud folder – contents not downloaded, not scanned".
  - Delete warning: "Deleting removes it from iCloud on all your devices and frees almost no space on this Mac."

### 1.12 The Trash
- **Issue:** Trashed items still occupy space until the Trash is emptied.
- **Locations:**
  - `~/.Trash` on the boot volume.
  - `/Volumes/<vol>/.Trashes/<uid>` on other volumes.
- **Evidence (M, stat only):**
  - `~/.Trash` has 40 entries.
  - It carries the ACL `group:everyone deny delete`, as do Desktop, Documents, Downloads, Library, Music and Pictures.
  - Its content is TCC-protected: listing requires FDA and fails with `EPERM` without it (R, widely reported; not listed here per the safety rules).
- **v1:** Scan it like any directory when it is under the root.
- **UI:**
  - Special "Trash" icon and node label.
  - Hint "Empty the Trash in Finder to free this space".
  - Later: "Empty Trash" via Finder Apple event (needs Automation consent).

---

## 2. macOS volumes, firmlinks and mount boundaries

### 2.1 Layout on this machine (M)

| mount point | dev (`st_dev`) | volume | flags (getmntinfo) | SPACEUSED | objects |
|---|---|---|---|---|---|
| `/` | 16777232 | disk3s3s1 System **snapshot** (sealed, rdonly) | rdonly, local, rootfs, snapshot (no dontbrowse) | 16.50 GB | 458,735 |
| `/System/Volumes/Data` | **16777232** (same!) | disk3s1 Data | local, dontbrowse, `f_flags_ext=MNT_EXT_ROOT_DATA_VOL` | 816.54 GB | 7,400,573 |
| `/System/Volumes/VM` | 16777233 | disk3s6 (swap, sleepimage) | dontbrowse, noexec | 0.00 GB | 0 |
| `/System/Volumes/Preboot` | 16777234 | disk3s4 | dontbrowse | 15.01 GB | 1,309 |
| `/System/Volumes/Update` | 16777230 | disk3s2 | dontbrowse | 1.02 GB | 494 |
| `/System/Volumes/Update/mnt1` | 16777231 | **disk3s3 = the System volume again** (update staging) | dontbrowse | 16.50 GB | 458,732 |
| `/System/Volumes/Update/SFR/mnt1` | 16777227 | disk2s1 (Recovery container) | dontbrowse | – | – |
| `/System/Volumes/{xarts,iSCPreboot,Hardware}` | 16777224/22/23 | ISC container disk1 | dontbrowse | ~0.01 GB each | – |
| `/System/Volumes/Data/home` | 889192451 | autofs `map auto_home` | **automounted**, dontbrowse, *not* local | – | – |
| `/dev` | 664207251 | devfs | dontbrowse | – | – |

- Container disk3 is 994.66 GB, with 850.4 GB in use by volumes: Data 815.1 + System 16.5 + Preboot 15.0 + Recovery 2.6 (unmounted) + Update 1.0 + VM 0.
- Nothing is mounted under `/Volumes` except the symlink `/Volumes/Macintosh HD -> /`.

### 2.2 The System/Data volume group shares one `st_dev`
- **Issue:**
  - Since 10.15, `/` (read-only System) and `/System/Volumes/Data` form a volume group with **one logical `st_dev`**, also exposed as `ATTR_CMN_DEVID`/`FSID` (S: `FSOPT_RETURN_REALDEV` docs: "the filesystem may return a common logical value").
  - Any "stay on device" rule (`du -x`, ncdu `-x`, dua `-x`, `FTS_XDEV`) therefore **descends into `/System/Volumes/Data`** when scanning `/` and counts the whole Data volume a second time: about 816 GB here, on a 1 TB disk. This is inferred from the measured dev ids; `/` was not scanned, by the safety rules.
- **Evidence (M):**
  - `stat` shows dev 16777232 for both `/` and `/System/Volumes/Data`.
  - `ATTR_CMNEXT_REALDEVID` distinguishes them: 16777235 for objects on System (`/`, `/System/Library`, `/usr/bin`, `/tmp` symlink) vs 16777232 for objects on Data (`/Users`, `/usr/local`, `/System/Library/Caches`).
  - `statfs("/").f_fsid` = 0x1000013 vs `statfs("/Users").f_fsid` = 0x1000010.
- **v1:** Mount-table based boundaries (§10), with `st_dev` only as a backstop.
- **UI:** none (it must simply be right).

### 2.3 Firmlinks
- **Issue:**
  - Directories on the System volume transparently redirect into the Data volume.
  - The list is in `/usr/share/firmlinks` as `SOURCE<TAB>DATA-RELATIVE-TARGET`. Here:
    - `/AppleInternal`, `/Applications`, `/Library`, `/Users`, `/Volumes`, `/cores`, `/opt`, `/pkg`, `/private`, `/usr/local`;
    - `/System/Library/{Caches,Assets,PreinstalledAssets,AssetsV2,PreinstalledAssetsV2,Speech}`;
    - `/System/Library/CoreServices/CoreTypes.bundle/Contents/Library`;
    - `/usr/libexec/cups`, `/usr/share/snmp`.
  - `/AppleInternal` and `/pkg` are listed but **do not exist** on `/`.
- **Evidence (M):**
  - `lstat` follows firmlinks: `/Users` and `/System/Volumes/Data/Users` both give `(16777232, 16991)`.
  - `getattrlistbulk` on the parent shows the firmlink object itself with `SF_FIRMLINK` (0x00800000) and a System-volume file id. Seen for `/usr/local`, `/System/Library/Caches` and the others; the bulk walker counted 3 firmlinks in `/usr` and 7 in `/System/Library`.
  - `ATTR_CMNEXT_NOFIRMLINKPATH` gives the canonical path, e.g. `/Users` → `/System/Volumes/Data/Users`.
  - `/System/Library` walked through its firmlinks totals 16.25 GiB, of which `AssetsV2` (on Data) is 5.94 GiB.
- **v1:** Traverse firmlinks as normal directories; they are not mount points and have the same dev. Record the `(dev, ino)` of every existing firmlink source; these are the Data-side targets. See §10 for their use.
- **UI:** none. Optionally a tooltip "Stored on the Data volume".

### 2.4 Separate system volumes under `/System/Volumes`
- **Issue:**
  - VM, Preboot, Update, xarts, iSCPreboot and Hardware are real separate volumes.
  - `Update/mnt1` re-mounts the **System volume itself** (M), so naive cross-mount traversal double counts it.
  - None of them is user-deletable.
- **v1:** Skip them; they are mount points. Show each as a non-expandable "System volume" leaf whose size comes from `ATTR_VOL_SPACEUSED`.
- **UI:**
  - Grey leaf "Preboot – 15.0 GB (system volume, not scanned)".
  - Or group them into a DaisyDisk-style "Hidden/system space" node (§8.4).

### 2.5 Data-volume content not reachable through firmlinks
- **Evidence (M, non-recursive `ls` of `/System/Volumes/Data`):**
  - Unreadable to non-root:
    - `.Spotlight-V100` → `EPERM`
    - `.fseventsd` → `EACCES`
    - `.DocumentRevisions-V100` → `EPERM`
  - Readable:
    - `.PreviousSystemInformation` (readable)
    - `.TemporaryItems`
    - `.com.apple.templatemigration.boot-install`
    - `MobileSoftwareUpdate` (readable)
    - `mnt`
    - `sw`
    - `pkg` (its firmlink source is missing)
  - Mount: `home` (autofs)
  - Firmlink-target containers: `System`, `usr`
- **v1:** Count these as the "Data volume remainder" when scanning `/` (§10). They are mostly unreadable and not user-deletable.
- **UI:** Node label "System/Volumes/Data (other system data)". Lock icons on unreadable children.

### 2.6 `/Volumes`: external disks, DMGs, network shares, Time Machine
- **Issue:**
  - Other filesystems: different dev, possibly slow (USB, network) or hanging (network).
  - Removable and network volumes are TCC categories that **prompt**.
  - Time Machine disks hold snapshot mounts (`/Volumes/.timemachine/...`) or old HFS+ `Backups.backupdb` with directory hard links. These are FDA-protected and massive double counting.
  - `/Volumes/Macintosh HD` is a symlink to `/`, harmless when symlinks are never followed.
- **v1:**
  - Never enter them unless the user picks one as root.
  - When the root is inside a volume where `MNT_LOCAL` is not set (network), warn "network volume – scan may be slow".
  - When the root's fstype is `hfs`, enable directory-hard-link dedupe: `ATTR_DIR_LINKCOUNT > 1` gives a visited set keyed by `(dev, ino)`, the same as Apple `du`'s `dirlinkchk`. Exclude `Backups.backupdb` by default.
- **UI:**
  - Start screen lists volumes (from getmntinfo, `dontbrowse` ones hidden) with used/total. Network volumes are marked.
  - Inside a scan, other volumes appear as leaves "Other volume – not scanned".

### 2.7 autofs
- **Issue:**
  - autofs maps such as `/System/Volumes/Data/home` (`auto_home`), `/Network/Servers`, or `/net` if enabled. `/net` is commented out in `/etc/auto_master` here.
  - Looking a name up inside a trigger can mount network filesystems and block.
- **Evidence (M, S):**
  - autofs appears in getmntinfo with `MNT_AUTOMOUNTED` and without `MNT_LOCAL`.
  - `setiopolicy_np(IOPOL_TYPE_VFS_TRIGGER_RESOLVE=5, IOPOL_SCOPE_PROCESS, IOPOL_VFS_TRIGGER_RESOLVE_OFF=1)` succeeds. Thread scope returns `EINVAL`.
  - xnu `vfs_context_can_resolve_triggers` honours it for user-space-resolved triggers.
- **v1:**
  - Set trigger-resolve OFF process-wide at startup.
  - Skip autofs mount points by path from the snapshot, **without** stat'ing them.
- **UI:** none, or a grey "Automount (not scanned)" leaf.

### 2.8 devfs (`/dev`)
- **Issue:** Device nodes report odd sizes, and devfs is a different filesystem.
- **v1:** Mount point → skip.
- **UI:** none.

### 2.9 Magic namespace directories `/.vol`, `/.nofollow`, `/.resolve`
- **Evidence (M):**
  - All three are empty directories on the sealed volume (nlink 2, 64 B).
  - But `/.nofollow/` with a trailing slash resolves to `/` (ino 2), and `/.nofollow/usr` resolves to `/usr`.
  - A walker only ever builds `/.nofollow/<name>` from `readdir` results (empty), so this is harmless today.
- **v1:** Hard-exclude `/.vol`, `/.nofollow`, `/.resolve` at the root of `/` anyway, at zero cost. The general ancestor-loop guard (§10) also catches it.
- **UI:** none.

### 2.10 Precise traversal answers
- **Scanning `/`: traverse**
  - The System volume.
  - The Data volume **via firmlinks**.
  - The Data-volume remainder: enter `/System/Volumes/Data`, but skip any directory whose `(dev, ino)` is a firmlink target.
- **Scanning `/`: skip**
  - `/System/Volumes/{VM,Preboot,Update(+mnt1,SFR/mnt1),xarts,iSCPreboot,Hardware}`
  - `/dev`
  - `/System/Volumes/Data/home`
  - everything mounted under `/Volumes`
  - any other mount
  - `/.vol`, `/.nofollow`, `/.resolve`
  - all symlinks (`/tmp`, `/var`, `/etc`, `/home`, `/Volumes/Macintosh HD`)
- **Scanning `/`: expected result**
  - Total ≈ SPACEUSED(System) + SPACEUSED(Data) = 16.50 + 816.54 = **833.0 GB** here, as an upper bound.
  - Reduced by: unreadable areas (root-only and TCC), blocks held only by snapshots, metadata and xattrs.
  - Increased by: clone overcounting.
- **Scanning `/System/Volumes/Data` directly:**
  - Root mount = Data. No firmlinks are involved; they live on the System volume.
  - Skip `home` (autofs). Skip anything mounted at `/Volumes/*`, which is also reachable here as `/System/Volumes/Data/Volumes/*`. That needs the alias rule below; the dev check is the backstop.
  - Total ≈ SPACEUSED(Data) = 816.5 GB minus unreadable parts.
- **Scanning `~` (`/Users/<me>`):**
  - Root mount = Data. Same dev rules.
  - Typically nothing is mounted below `~` except user FUSE/sshfs mounts; skip them.
  - The issues here are TCC (§5), iCloud and File Provider dataless content (§1.11) and `~/.Trash` (FDA).
  - No reliable total estimate exists; show indeterminate progress (§8).
- **Alias rule (from gdu's `getMountPointAlias`, S):** for every mount point `P` below a firmlink source `S`, also register `/System/Volumes/Data/<target-of-S>/<rest of P>` as a mount-point path. Example: `/Volumes/USB` → `/System/Volumes/Data/Volumes/USB`.

### 2.11 Detecting filesystem type and mount boundaries cheaply

| need | macOS | Linux |
|---|---|---|
| mount snapshot (no blocking) | `getmntinfo(&buf, MNT_NOWAIT)`: cached stats, never calls into a dead network fs (S, M) | read `/proc/self/mountinfo` text: mount id, parent id, `maj:min`, root-within-fs, mount point (octal escapes `\040` `\011` `\012` `\134`), fstype, source |
| fs type | `statfs.f_fstypename` (`apfs`, `hfs`, `smbfs`, `nfs`, `autofs`, `devfs`, `msdos`, `exfat`, `macfuse`...) | mountinfo fstype; or `fstatfs.f_type` magic (`PROC_SUPER_MAGIC` 0x9fa0, `SYSFS_MAGIC`, `TMPFS_MAGIC`, `NFS_SUPER_MAGIC`, `FUSE_SUPER_MAGIC`, `OVERLAYFS_SUPER_MAGIC`, `BTRFS_SUPER_MAGIC`...) |
| local vs network | `f_flags & MNT_LOCAL` | by fstype list (nfs, nfs4, cifs, smb3, 9p, ceph, glusterfs, lustre, afs, fuse.*) |
| automount | `f_flags & MNT_AUTOMOUNTED`, fstype `autofs` | fstype `autofs`; use `AT_NO_AUTOMOUNT` (all of `stat`/`lstat`/`fstatat` already act as if it were set; S, man statx) |
| hidden system mounts | `f_flags & MNT_DONTBROWSE`: all of `/System/Volumes/*`, devfs, autofs (M) | n/a |
| root Data volume | `f_flags_ext & MNT_EXT_ROOT_DATA_VOL` (0x1) (S, M) | n/a |
| FSKit (user-space) fs | `f_flags_ext & MNT_EXT_FSKIT` (0x2) | n/a |
| is this dir a mount point (no I/O into the mounted fs) | `getattrlistbulk` returns `ATTR_DIR_MOUNTSTATUS & DIR_MNTSTATUS_MNTPOINT` with the *covered* dir's attributes (S, M: Data/VM/Preboot/... flagged) | `statx(..., STATX_MNT_ID)`: `stx_mnt_id` differs from the parent (5.8+); `STATX_ATTR_MOUNT_ROOT` (5.8+); else path ∈ mountinfo set |
| real volume inside a group | `ATTR_CMNEXT_REALDEVID` | n/a |
| btrfs subvolume id | n/a | `STATX_SUBVOL`/`stx_subvol` (6.10+) |

**Pitfall (M):** `statfs` on APFS reports the **container**: `f_blocks`=242,837,545 × 4096 = 994.66 GB, and `f_bfree`=`f_bavail` = container free, for *every* volume. So `(f_blocks-f_bfree)*f_bsize` = 851.85 GB on every APFS volume. Apple's `df` uses `getattrlist(ATTR_VOL_INFO|ATTR_VOL_SPACEUSED)` per volume (S: file_cmds `df.c`).

**libc 0.2.191 coverage (S, grep of the crate):**
- Present for macOS:
  - `getmntinfo`, `getfsstat`, `statfs` (with `f_flags_ext`), `fstatfs`
  - `getattrlist`, `fgetattrlist`, `getattrlistat`, `getattrlistbulk`
  - `attrlist`, `attribute_set_t`, `attrreference_t`
  - all `ATTR_CMN_*`/`ATTR_VOL_*`/`ATTR_DIR_*`/`ATTR_FILE_*`, `ATTR_CMNEXT_{PRIVATESIZE,LINKID,NOFIRMLINKPATH,REALDEVID,REALFSID,CLONEID,EXT_FLAGS,RECURSIVE_GENCOUNT}`
  - `FSOPT_*`, `DIR_MNTSTATUS_MNTPOINT`, `VOL_CAP_*` (partial)
  - `UF_COMPRESSED`, `UF_HIDDEN`
  - `MNT_LOCAL`, `MNT_AUTOMOUNTED`, `MNT_DONTBROWSE`, `MNT_ROOTFS`, `MNT_SNAPSHOT`, `MNT_QUARANTINE`, `MNT_NOWAIT`
  - `O_NOFOLLOW_ANY`, `clonefile`, `sysctlbyname`
- Missing on macOS; declare locally, values from the MacOSX27 SDK:

  ```rust
  const ATTR_CMN_ERROR: u32 = 0x2000_0000;
  const ATTR_CMNEXT_CLONE_REFCNT: u32 = 0x1000;
  const EF_MAY_SHARE_BLOCKS: u64 = 0x1; const EF_IS_PURGEABLE: u64 = 0x8;
  const EF_IS_SPARSE: u64 = 0x10; const EF_IS_SYNC_ROOT: u64 = 0x4; const EF_SHARES_ALL_BLOCKS: u64 = 0x40;
  const SF_RESTRICTED: u32 = 0x0008_0000; const SF_NOUNLINK: u32 = 0x0010_0000;
  const SF_FIRMLINK: u32 = 0x0080_0000; const SF_DATALESS: u32 = 0x4000_0000;
  const MNT_EXT_ROOT_DATA_VOL: u32 = 0x1; const MNT_STRICTATIME: u32 = 0x8000_0000;
  const VOL_CAP_FMT_CLONE_MAPPING: u32 = 0x0400_0000;
  extern "C" { fn getiopolicy_np(t: c_int, s: c_int) -> c_int; fn setiopolicy_np(t: c_int, s: c_int, p: c_int) -> c_int; }
  // IOPOL_TYPE_DISK 0, _VFS_ATIME_UPDATES 2, _VFS_MATERIALIZE_DATALESS_FILES 3, _VFS_TRIGGER_RESOLVE 5
  // IOPOL_SCOPE_PROCESS 0, _THREAD 1; *_OFF = 1; IOPOL_UTILITY 4
  ```

  `getattrlistbulk` records start 8-byte aligned. Pack order:
  1. `RETURNED_ATTRS`, then `ERROR`, then `NAME`;
  2. common attrs in bit order;
  3. dir, then file;
  4. extended-common attrs (in the forkattr slot).

  Volume attrs follow xnu order, which is *not* pure bit order: `SPACEUSED` comes right after `SPACEAVAIL` (S, verified with the probe).
- Present for Linux: `statx`, `STATX_MNT_ID`, `STATX_MNT_ID_UNIQUE`, `STATX_SUBVOL`, `STATX_ATTR_MOUNT_ROOT`, `AT_NO_AUTOMOUNT`, `AT_STATX_DONT_SYNC`, `fstatfs`, most `*_MAGIC` constants, `setmntent`/`getmntent`, `openat2`, `RESOLVE_NO_XDEV`.
- Missing on Linux: `SQUASHFS_MAGIC` (0x73717368), `CIFS_MAGIC_NUMBER` (0xFF534D42), `SMB2_MAGIC_NUMBER` (0xFE534D42).

No extra crate is needed. `rustix` is an option for Linux `statx` and `openat`. `sysinfo` lists disks but hides the flags we need.

---

## 3. Linux (secondary)

### 3.1 Pseudo filesystems
- **Issue:**
  - `/proc`, `/sys`, `/dev` (devtmpfs, devpts), `/run` (tmpfs), cgroup/cgroup2, debugfs, tracefs, securityfs, pstore, bpf, configfs, mqueue, hugetlbfs, binfmt_misc, efivarfs, fusectl, nsfs, rpc_pipefs.
  - Meaningless sizes (e.g. `/proc/kcore` typically reports ~128 TiB apparent on x86_64), endless churn, and for some, reads with side effects.
- **How others do it (S):**
  - gdu and dua ignore `/proc,/dev,/sys,/run` by default on Linux.
  - ncdu `--exclude-kernfs` checks `fstatfs.f_type` against BINFMTFS, BPF_FS, CGROUP, CGROUP2, DEBUGFS, DEVPTS, PROC, PSTOREFS, SECURITYFS, SELINUX, SYSFS, TRACEFS.
- **v1:** These are mount points of other filesystems, so the mount rule skips them. Additionally hard-exclude `/proc`, `/sys`, `/dev`, `/run` when the root is `/`.
- **UI:** grey "system (virtual) – not scanned" leaves.

### 3.2 tmpfs and ramfs
- **Issue:** RAM-backed, not disk.
- **v1:** Treat as other filesystems (skip unless chosen as root). `/tmp` on tmpfs (Fedora, Arch) then shows as "other volume".
- **UI:** none.

### 3.3 Snap squashfs mounts
- **Issue:** `/snap/<name>/<rev>` are read-only squashfs loop mounts of `/var/lib/snapd/snaps/<name>_<rev>.snap`. Traversing both counts the data twice: once compressed as the `.snap` file, once expanded.
- **v1:** Skip squashfs mounts (mount rule plus fstype). The real cost is the `.snap` file, which is counted.
- **UI:** none.
- **Note:** gdu also skips `/snap/` when listing devices (S).

### 3.4 Bind mounts
- **Issue:**
  - Same filesystem mounted twice; **same `st_dev`**. A dev check cannot see the boundary.
  - It can even create loops, e.g. a bind of `/` under `/mnt`.
  - In mountinfo, field 4 (root) ≠ `/` and a duplicate `maj:min` gives it away.
- **v1:**
  - Mount-point rule: every mount point in the snapshot other than the root's mount is a boundary.
  - Prefer `statx(STATX_MNT_ID)` for the check: `stx_mnt_id` changes at any mount, including binds (5.8+).
  - Fall back to path ∈ mountinfo set.
  - Plus the ancestor `(dev, ino)` loop guard.
- **UI:** "Mount point – not scanned".

### 3.5 btrfs subvolumes and snapshots
- **Issue:**
  - Every subvolume gets its own anonymous `st_dev`, even when it is not a separate mount (R: RH bugzilla 711881 "we create a fake super for every subvol"; LWN 866709). So `du -x`/`find -xdev`-style checks stop at subvolumes.
  - That is *desirable* for snapper (`/.snapshots/*`) and docker/machines subvolumes, which share extents (CoW). Counting them multiplies totals.
  - `statfs.f_fsid` also differs per subvolume (R).
  - btrfs `statfs.f_files` is 0 (not verified here), so there is no item count for progress.
- **v1:**
  - Keep the dev check: subvolume = boundary leaf "btrfs subvolume (not scanned)", with a per-node action "Scan into".
  - If the user opts to include subvolumes, still exclude `.snapshots`.
  - Reflinks (`cp --reflink`) behave like APFS clones; there is no cheap detection (FIEMAP `SHARED` flag per extent is expensive). Use the same "may free less" wording.
- **UI:** as stated.

### 3.6 overlayfs (Docker, Podman, live media)
- **Issue:**
  - `.../overlay2/<id>/merged` mounts duplicate content that also lives in the lower and upper `diff` dirs.
  - `st_dev` and inode numbers inside overlays are inconsistent: directories show the overlay dev, files may show the underlying one unless `xino`. That breaks dev checks and hard-link keys.
- **v1:** Skip overlay mounts unless the root is inside one. Inside an overlay root, use `stx_mnt_id` (not `st_dev`) for boundaries and disable hard-link dedupe for files whose dev ≠ the root's dev.
- **UI:** none.

### 3.7 Network and FUSE filesystems
- **Issue:**
  - nfs, nfs4, cifs/smb3, sshfs (`fuse.sshfs`), rclone, gvfs (`fuse.gvfsd-fuse` under `/run/user/<uid>/gvfs`), s3fs, 9p, ceph, glusterfs.
  - Slow. With a dead server, a `stat` on a hard NFS mount blocks in uninterruptible D state: the thread hangs forever and cannot be cancelled.
- **v1:**
  - Classify mounts from the mountinfo **text**.
  - Never `stat`/`statfs` a network mount you are not scanning.
  - Scan one only when chosen as root.
  - Use `AT_STATX_DONT_SYNC` there.
  - Run scans on worker threads so the UI never blocks; on cancel, abandon blocked threads.
- **UI:** "Network volume – not scanned" leaves. A warning when starting a scan inside one.

### 3.8 ZFS
- **Issue:**
  - Datasets are separate mounts, so the mount rule applies.
  - `<dataset>/.zfs/snapshot/*` (normally hidden, `snapdir=hidden`) automounts snapshots on access.
- **v1:** Hard-exclude `.zfs` at dataset roots.
- **UI:** none.

### 3.9 Scanning `/` on Linux
- **v1:**
  - Root mount only. Other local disk filesystems (separate `/home`, `/boot`, `/boot/efi`, data disks) appear as leaves with size from `statvfs` (cheap for local block-device filesystems). The user can opt in per leaf.
  - Expected total ≈ `(f_blocks - f_bfree) * f_frsize` of the root fs.
  - Expected items ≈ `f_files - f_ffree` (ext4, xfs); unavailable on btrfs.
- **UI:** as stated.

### 3.10 What the existing tools do (S)

| | ncdu 2.9.2 | gdu (2026-10) | dua-cli (2026-11) |
|---|---|---|---|
| size | `st_blocks*512`, apparent optional | `st_blocks*512` | `st_blocks*512`; macOS `ATTR_FILE_ALLOCSIZE` |
| one-fs | `-x`: `st_dev` vs parent | `-x`: mount table → ignore nested mount points (+ macOS `/System/Volumes/Data` alias) | `-x`: `st_dev` vs root |
| default excludes | `--exclude-kernfs` opt-in (f_type list) | `/proc,/dev,/sys,/run` | Linux `/proc,/dev,/sys,/run` |
| hard links | unique/shared per dir after scan | first-seen, keyed by inode only, flag `H` | `(dev,ino)` with remaining-link countdown |
| macOS firmlinks / volume group | **BUGS: counted multiple times** | handled via mount table | not special-cased |
| APFS clones | no | no | opt-in `--deduplicate-apfs-clones` (CLONEID, `EF_MAY_SHARE_BLOCKS`) |
| traversal | openat-relative, multi-thread | Go, path-based | jwalk / `getattrlistbulk` on macOS |
| caches | `--exclude-caches` (CACHEDIR.TAG) | – | – |

---

## 4. Windows (tertiary: compile and behave sanely)

### 4.1 Size on disk
- **Issue and std facts (S):**
  - `std` gives only the logical length.
  - `DirEntry::metadata()` comes from `FindFirstFileExW` data, with no per-file open; that is cheap.
  - `MetadataExt::file_attributes()` and `file_size()` are stable.
  - `volume_serial_number`, `number_of_links` and `file_index` are **unstable** (`windows_by_handle`).
- **Options:**
  - **(a) v1:** logical size, optionally rounded up to the cluster size from `GetDiskFreeSpaceW`.
  - **(b)** `GetCompressedFileSizeW`. It returns the on-disk size only for compressed or sparse files, otherwise "the actual file size, the same as `GetFileSize`". It follows symlinks and needs one call per file (S: MS docs). The `filesize` crate does this *after* `canonicalize`, which resolves junctions; avoid that.
  - **(c) Later:** enumerate directories with `GetFileInformationByHandleEx(dir, FileFullDirectoryInfo / FileIdExtdDirectoryInfo)` via `windows-sys`. This returns `AllocationSize`, `EndOfFile`, attributes, file id and reparse tag per entry without opening files.
- **v1:** (a).
- **UI:** label the column "Size" instead of "Size on disk" on Windows v1.

### 4.2 Junctions, symlinks, mount points, reparse points
- **Issue:**
  - Junctions create loops and double counts: `C:\Documents and Settings` → `C:\Users`; `...\AppData\Local\Application Data` → itself.
  - Volume mount points are junctions too.
- **Evidence (S, std 1.93):** `FileType::is_symlink()` is true for any reparse point whose tag has the **name-surrogate** bit (0x20000000), i.e. symlinks **and** junctions (`IO_REPARSE_TAG_MOUNT_POINT`). `is_dir()` is then false. So "descend only if `file_type().is_dir()`" never enters them.
- **Cloud placeholders:**
  - OneDrive placeholders use non-surrogate tags, so they look like normal files and dirs.
  - Detect them through attributes: `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` 0x400000, `RECALL_ON_OPEN` 0x40000, `OFFLINE` 0x1000.
  - Do not open those files. Show a cloud badge with size on disk ≈ 0.
- **v1:** as stated.
- **UI:** cloud badge as above.

### 4.3 Hard links
- **Issue:** `C:\Windows\WinSxS` is hard-linked into System32, so `C:\Windows` looks much bigger than it is.
- **v1:** No dedupe (needs unstable std or a per-file `GetFileInformationByHandle`).
- **UI:** Tooltip on `C:\Windows` saying sizes may double count hard links.

### 4.4 Long paths
- **Evidence (S):** std converts long paths to `\\?\` verbatim form automatically (`maybe_verbatim`, legacy limit 248/260).
- **v1:** No action.
- **UI:** none.

### 4.5 Other entries
- `System Volume Information`: access denied; show as unreadable.
- `$Recycle.Bin\<SID>`: the Trash.
- `pagefile.sys`, `hiberfil.sys`, `swapfile.sys`: count them, badge them "system", never offer delete.

### 4.6 cfg-gating strategy
One `platform` module with identical signatures per OS:

```rust
pub struct EntryInfo { kind: Kind, apparent: u64, allocated: u64, dev: u64, ino: u64, nlink: u64, flags: EntryFlags }
pub fn entry_info(meta: &std::fs::Metadata) -> EntryInfo;   // unix: blocks()*512, dev(), ino(), nlink(); macos: + st_flags()
                                                            // windows: len(), dev=0, ino=0, nlink=1, flags from file_attributes()
pub fn init_scan_thread();                                  // macos: setiopolicy_np(thread); others: no-op
pub fn init_process();                                      // macos: trigger-resolve off, atime off, raise RLIMIT_NOFILE; unix: rlimit
pub fn mount_snapshot() -> MountTable;                      // macos: getmntinfo(MNT_NOWAIT)+firmlinks; linux: /proc/self/mountinfo; windows: empty
pub fn volume_usage(root: &Path) -> Option<VolumeUsage>;    // macos: ATTR_VOL_SPACEUSED/OBJCOUNT; linux: statvfs; windows: GetDiskFreeSpaceExW or None
pub fn classify_error(e: &io::Error) -> ErrorKind;          // EACCES/EPERM/EDEADLK/ENOENT/ENAMETOOLONG; windows: ERROR_ACCESS_DENIED etc.
pub fn delete_guard(path: &Path) -> Result<(), Refusal>;    // per-OS protected list
```

- File layout:
  - `#[cfg(target_os = "macos")] mod macos;`
  - `#[cfg(target_os = "linux")] mod linux;`
  - `#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))] mod unix_generic;`
  - `#[cfg(windows)] mod windows;`
  - `#[cfg(not(any(unix, windows)))] mod fallback;` (apparent size only).
- Keep the core aggregation and tree code platform-free.
- CI matrix (macOS, Ubuntu, Windows). Locally `cargo check --target x86_64-unknown-linux-gnu` / `x86_64-pc-windows-msvc` works without a linker, but needs `rustup target add` (not installed on this machine).

---

## 5. Permissions, macOS TCC and Full Disk Access

### 5.1 Error classes
- **Evidence (M, `opendir`):**

  | errno | example paths | meaning |
  |---|---|---|
  | `EACCES` 13 | `/private/var/root`, `/private/var/db/sudo`, `/usr/sbin/authserver`, `/System/Volumes/Data/.fseventsd`, `/System/Library/Caches/com.apple.coresymbolicationd` | plain Unix permissions |
  | `EPERM` 1 | `/Library/Application Support/com.apple.TCC`, `/System/Volumes/Data/.Spotlight-V100`, `.DocumentRevisions-V100`, `/System/Library/AssetsV2/com_apple_MobileAsset_UAF_*` | policy (TCC/sandbox/datavault) is checked before Unix permissions |

  - `std::io::ErrorKind` is `PermissionDenied` for **both**; use `raw_os_error()`.
  - Error mix seen: `/System/Library` had 62 `EACCES` and 4 `EPERM` dirs; `/usr` had 1 `EACCES`.
- **v1:**
  - Mark the directory as unreadable.
  - Its known size is 0 plus its own blocks.
  - Propagate an "incomplete" flag to ancestors.
  - Don't spam: no dialogs per error. Keep a collapsible list.
- **UI:**
  - Lock icon "Permission denied (needs administrator)" for `EACCES`.
  - Shield icon "Blocked by macOS privacy protection" for `EPERM`.
  - Ancestors show "≥ 12.3 GB" or "12.3 GB+" with an "incomplete" dot.
  - Top banner when any `EPERM` under `~` occurred and FDA is missing (§5.4).

### 5.2 TCC behaviour that matters for a scanner
- **Prompting categories.** One dialog per category per app, remembered under System Settings > Privacy & Security > Files & Folders:
  - Desktop, Documents, Downloads, network volumes, removable volumes (Apple Platform Security; R/S);
  - iCloud Drive and every **File Provider domain** (`~/Library/CloudStorage/*`): one prompt per account on macOS 14+ (`kTCCServiceFileProviderDomain`; R).
- **App Data (macOS 14+).** Touching another developer's container in `~/Library/Containers`, and on macOS 15+ `~/Library/Group Containers`, prompts "…would like to access data from other apps".
  - Per WWDC23 the permission lasts **only while the app runs**, so it re-prompts every launch.
  - FDA overrides it (R: WWDC23 10053, WWDC24 10123, lapcatsoftware).
- **FDA-only locations never prompt; they silently fail with `EPERM`:** Mail, Messages, Safari, Cookies, HomeKit, Suggestions, the TCC db, Time Machine backups, `~/.Trash` contents, Photos library internals (R; examples of `EPERM` measured above).
- **Access blocks while a dialog is up.** Parallel threads hitting Desktop, Documents and Downloads simultaneously stack three dialogs. Design for this:
  - Ask for FDA first.
  - Or touch prompting folders sequentially from one thread before starting parallel work.
- **Identity (R, multiple 2026 reports):**
  - Grants are keyed to the code signature.
  - An unsigned or ad-hoc build gets a cdhash identity that changes on every rebuild, so grants vanish.
  - `cargo run` from a terminal makes the **terminal app** the responsible process: its grants apply and its name appears in dialogs.
  - For TCC testing, bundle the app and sign it with a stable certificate (self-signed or Developer ID).
- **Info.plist purpose strings.** Ship all of:
  - `NSDesktopFolderUsageDescription`, `NSDocumentsFolderUsageDescription`, `NSDownloadsFolderUsageDescription`
  - `NSNetworkVolumesUsageDescription`, `NSRemovableVolumesUsageDescription`
  - `NSFileProviderDomainUsageDescription`, `NSAppDataUsageDescription`

  A helper without usage strings saw *no* dialog and silent `EPERM` (R: Orca PR).

### 5.3 Full Disk Access: deep link and detection
- **Deep link (S, M):**
  - MacOSX27 SDK `EndpointSecurity/ESClient.h` documents:
    - `x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles` for macOS 13 and later;
    - `...com.apple.preference.security?Privacy_AllFiles` until macOS 12. Apple: "use the new URL in macOS 13 as the old one may stop working".
  - On this macOS 26.7 the System Settings extension `com.apple.settings.PrivacySecurity.extension` has `allowsXAppleSystemPreferencesURLScheme = true` and `legacyBundleIdentifier = com.apple.preference.security`. Its search terms define the anchors `Privacy_AllFiles` ("Allow applications to access all user files") and `Privacy_AppBundles` (App Management).
  - Not launched here: it would open UI.
  - Open it with NSWorkspace (objc2-app-kit) or `open <url>`. Some reports say only NSWorkspace worked in early Ventura betas (R). Verify once manually.
- **Detection heuristic (no prompt is ever shown for FDA-only paths):**
  - `opendir("/Library/Application Support/com.apple.TCC")` succeeds ⇔ FDA granted. Here: `EPERM`, consistent with no FDA.
  - This is a heuristic. Apple calls `es_new_client` "the only supported way", but that is only for ES clients.
- **v1 flow:**
  1. Start screen with a "Grant Full Disk Access (recommended)" card for `~`/`/` scans, showing status via the heuristic.
  2. Button opens the deep link; text: "add dir-stat, then quit & reopen" (TCC changes need a relaunch).
  3. Without FDA, the scan still works, but expect prompts. Offer "Skip privacy-protected folders" (pre-excluded, never touched):
     - `~/Desktop`, `~/Documents`, `~/Downloads`
     - `~/Library/Mobile Documents`, `~/Library/CloudStorage`
     - `~/Library/Containers`, `~/Library/Group Containers`
     - `~/.Trash`
     - `/Volumes/*` (network, removable)
- **UI:** as in the flow above.

---

## 6. Names, paths and bundles

### 6.1 Encoding
- **Evidence (M):**
  - APFS rejects non-UTF-8 names with `EILSEQ` (92).
  - Linux ext4 accepts arbitrary bytes except `/` and NUL.
  - Windows names are UTF-16 with possible lone surrogates (WTF-8 in `OsString`).
- **v1:**
  - Store each node's name as `OsString`/`Box<OsStr>` (only names, not full paths).
  - Build `PathBuf`s by walking parents.
  - Display with `to_string_lossy()`; sort with a lossy-lowercase key.
  - Never round-trip through `String` for actions (delete, reveal).

### 6.2 Case and normalization
- **Evidence (M):** The Data volume is case-insensitive (`casetest.TXT` finds `CaseTest.txt`) and Unicode-normalization-insensitive (NFC `café` finds an NFD-created file). `readdir` returns the stored NFD bytes `63 61 66 65 cc 81`.
- **v1:**
  - Tree keys are the exact bytes from `readdir`.
  - Never lowercase or normalize them.
  - Resolve user exclusions to `(dev, ino)` at scan start, which is robust to case and normalization. Compare mount points by exact path (kernel-provided strings).
  - Search box: compare NFC-normalized, case-folded strings (`unicode-normalization` crate).

### 6.3 NAME_MAX
- **Evidence (M):** 255 counts *characters*, not bytes: a 255 × "ä" name (510 bytes) is accepted; 256 ASCII chars fail `ENAMETOOLONG`.
- **v1:** Never assume names ≤ 255 bytes.

### 6.4 PATH_MAX (1024 on macOS, 4096 on Linux)
- **Evidence (M):**
  - With a 1,663-byte path, `std::fs::symlink_metadata` and `read_dir` fail `ENAMETOOLONG` (63). `du` also fails ("File name too long").
  - `fstatat(dirfd, "leaf")` and the openat-based bulk walker succeed.
  - Longest real paths seen: 269 B (`/Applications`), 363 B (`/System/Library`).
- **v1:** std path-based walking is acceptable; show `ENAMETOOLONG` dirs as "Path too long – not scanned".
- **Later:** openat/fdopendir-relative traversal (ncdu does this), which also hardens against renames during a scan.
- **UI:** Middle-ellipsize long paths.

### 6.5 Hidden items
- **Evidence (M):** `~/Library` carries `UF_HIDDEN` (`ls -lO` shows "hidden"). It is often the biggest folder in `~`.
- **v1:** Always count dotfiles and `UF_HIDDEN` items.
- **UI:** Optionally show them dimmed with a toggle "show hidden" that affects display only, never totals.

### 6.6 Bundles and packages
- **Issue:**
  - Bundles and packages are ordinary directories to the filesystem.
  - Finder shows them as single files, and users think of `.app`, `.photoslibrary`, `.xcarchive`, `.sparsebundle` and VM bundles as one thing.
- **Evidence (M):**
  - CoreTypes declares 51 extensions conforming to `com.apple.package`: `app appex bundle dext dsym kext mdimporter plugin prefpane qlgenerator saver systemextension xpc pkg mpkg rtfd pages numbers key band fcpproj imovielibrary mlpackage pkpass ...`.
  - Apps declare more: `photoslibrary`, `xcarchive`, `xcodeproj`, `xcworkspace`, `sparsebundle`, `fcpbundle`, `logicx`, `musiclibrary`, `tvlibrary`, `vmwarevm`, `pvm`, `utm`, `framework` (a bundle).
- **v1:**
  - Scan into them always.
  - Badge them by extension list, case-insensitive; for `.app`, also require `Contents/Info.plist`.
  - Offer "Reveal in Finder" and delete as a unit.
- **Later:** `NSURLIsPackageKey` via objc2 is exact.
- **UI:** Package icon, expandable, but collapsed by default. Inside `.photoslibrary`, `EPERM` without FDA is expected.

---

## 7. Robustness during the scan

### 7.1 Things change underneath you
- **Issue:**
  - `readdir` returned a name but `lstat` gives `ENOENT`: the file vanished.
  - `ENOTDIR`: a directory was replaced by a file.
  - `ESTALE` on NFS; `EINTR`.
  - Sizes grow while scanning (logs, caches, our own build dirs).
- **v1:**
  - `ENOENT`/`ENOTDIR` → drop silently and count `vanished` in scan stats.
  - `EINTR` → retry.
  - Totals are a snapshot approximation.
- **UI:** "Rescan folder" action; "Scanned at 17:41" timestamp.

### 7.2 File descriptors
- **Evidence (M):** `launchctl limit maxfiles` gives **256 soft** (unlimited hard); `kern.maxfilesperproc` = 245,760. A Finder-launched app starts with 256 fds, and iced/wgpu use some.
- **v1:**
  - At startup: `setrlimit(RLIMIT_NOFILE, min(hard, kern.maxfilesperproc, 10240))`.
  - Read a directory fully, close it, then descend.
  - Bounded worker count.
  - Treat `EMFILE` as retryable with backoff.

### 7.3 Side effects and hangs
- **Issue:** A scanner must not download, mount, wake disks, or update atimes.
- **v1 (macOS):**
  - Materialize OFF (thread).
  - Trigger resolve OFF (process).
  - `IOPOL_TYPE_VFS_ATIME_UPDATES` OFF (process or thread). All three verified to succeed (M). Thread scope is not supported for TRIGGER_RESOLVE (`EINVAL`).
  - Optional `IOPOL_TYPE_DISK = IOPOL_UTILITY` on scan threads (thread scope OK, M) for background rescans.
- **v1 (all OSes):**
  - Never open files.
  - Never stat skipped mount points.
  - Mount snapshot via non-blocking APIs.
  - Workers are detached so the UI survives a stuck syscall.

### 7.4 Loops
- **Issue:**
  - Symlinks are never followed, so there are no symlink loops.
  - Remaining loop sources: bind mounts (same dev), HFS+ directory hard links, `/.nofollow/`-style magic paths, buggy network filesystems.
- **v1:** Each queued directory carries its parent chain. Refuse to enter a directory whose `(dev, ino)` equals an ancestor's (O(depth); depth ≤ 22 observed). Show it as "Loop – skipped".

---

## 8. Volume capacity and progress estimation

### 8.1 Which numbers to use (M)
- **Total and free:** `statfs` `f_blocks*f_bsize` and `f_bavail*f_bsize`. On APFS these are container values: 994.66 GB / 142.8 GB, the same for all volumes.
- **Used by this volume:**
  - APFS: `getattrlist(ATTR_VOL_INFO|ATTR_VOL_SPACEUSED)` on the mount point (System 16.50 GB, Data 816.54 GB).
  - Other filesystems: `(f_blocks - f_bfree) * f_bsize`.
- **Object counts:** `ATTR_VOL_OBJCOUNT`/`FILECOUNT`/`DIRCOUNT`.
  - System: 458,735 objects (272,094 files, 170,576 dirs).
  - Data: 7,400,573 objects (5,942,673 files, 1,217,352 dirs).
  - These are `u32`. Fall back to `f_files - f_ffree`: 458,735 / 7,400,388 here; ext4 and xfs work, btrfs gives 0.

### 8.2 Best approximation of "bytes and items the scan will find"

| root | expected bytes | expected items | quality |
|---|---|---|---|
| `/` (macOS) | SPACEUSED(System)+SPACEUSED(Data) = 833.0 GB | OBJCOUNT sum ≈ 7.86 M | good upper bound. Clones push bytes up; unreadable, snapshot and metadata push them down. |
| a volume root (`/System/Volumes/Data`, `/Volumes/X`, Linux mount point) | that volume's used | its object/inode count | good |
| any subdirectory (`~`, `~/dev`) | unknown | unknown | none: show counters, or the previous scan's totals as the estimate |

- **v1:**
  - Progress bar driven by **items scanned / expected items**. Items progress is far smoother than bytes, which is dominated by a few huge files.
  - Capped at 99 % until the walk completes.
  - Indeterminate mode with live counters ("1.2 M items · 214 GB · 3 errors · current: ~/Library/Developer/…") when no estimate exists.

### 8.3 Linux and Windows equivalents
- **Linux:** `statvfs` `(f_blocks-f_bfree)*f_frsize` and `f_files-f_ffree`.
- **Windows:** `GetDiskFreeSpaceExW` (total − free); no item counts, so use indeterminate progress.

### 8.4 "Hidden / not scanned" node
- For volume-root scans, add a synthetic child: `max(0, expected_bytes − scanned_bytes)`.
- Label: "Not visible to this scan: system data, snapshots, unreadable folders, metadata". Break it down where known: other system volumes, unreadable dirs count, purgeable.
- This is DaisyDisk's "hidden space" pattern (R).

---

## 9. Deleting from the app

### 9.1 Mechanism
- **Library:** `trash` 5.2.9 (S). Its macOS default `DeleteMethod::Finder` runs `osascript 'tell application "Finder" to delete …'`:
  - Automation consent prompt ("dir-stat wants to control Finder"; needs `NSAppleEventsUsageDescription`);
  - slower;
  - plays the sound;
  - supports "Put Back".
- **v1:**
  - Use `trash::macos::TrashContextExtMacos::set_delete_method(DeleteMethod::NsFileManager)`. This is `-[NSFileManager trashItemAtURL:resultingItemURL:error:]`: no extra permission, faster, no "Put Back" (macOS bug noted by the crate).
  - The crate canonicalizes only the **parent**, so a symlink itself is trashed, not its target (S). `resultingItemURL` may be a renamed item (S: NSFileManager.h).
  - Fallback on permission failures: offer "Move to Trash with Finder". Finder can ask for an admin password; this needs Automation consent once.
- **Linux:** the same crate implements the FreeDesktop trash (`$XDG_DATA_HOME/Trash`, `$topdir/.Trash-$uid`).
- **Windows:** the same crate uses the Recycle Bin.

### 9.2 What cannot or should not be trashed
- **SIP / sealed system volume:**
  - `/System` (except firmlinked Data parts), `/usr` (except `/usr/local`), `/bin`, `/sbin`.
  - Read-only mount → `EROFS`/`EPERM`.
  - Items flagged `SF_RESTRICTED`/`SF_NOUNLINK`/`SF_IMMUTABLE`/`UF_IMMUTABLE`. All are visible in `st_flags` (M: `/System/Library` restricted, `/Users` sunlnk).
- **ACL "everyone deny delete":** the home folder's standard folders: Desktop, Documents, Downloads, Library, Movies, Music, Pictures, `.Trash` (M).
- **App Management (macOS 13+):**
  - Modifying or deleting another developer's `.app` needs "App Management" (anchor `Privacy_AppBundles`, M); otherwise the user sees "dir-stat was prevented from modifying apps".
  - App Store apps are root-owned.
  - The Finder fallback is the practical path.
- **Other apps' containers:** App Data prompt.
- **Root-owned items:** `EACCES` (NSFileManager does not authenticate).
- **Network shares:** may have no Trash. On failure, offer "Delete immediately…" behind a second explicit confirmation.

### 9.3 Hard safety guards (refuse with an explanation)
- **Never delete:**
  - the scan root itself;
  - any ancestor of the scan root;
  - any mount point or volume root (mount table), anything under `/System/Volumes/*`, and anything on a read-only mount;
  - `/`.
- **macOS:**
  - `/System`, `/Library`, `/Applications` (the folder; apps inside are allowed), `/Users`, `/private` and `/private/{etc,var,tmp}`;
  - `/usr`, `/bin`, `/sbin`, `/opt`, `/cores`, `/Volumes`, `/dev`;
  - symlinks `/etc`, `/var`, `/tmp`;
  - `~`;
  - the standard home folders themselves: `~/Library`, `~/Desktop`, `~/Documents`, `~/Downloads`, `~/Movies`, `~/Music`, `~/Pictures`, `~/Public`, `~/.Trash`;
  - also anything with the flags or ACL above.
- **Linux:**
  - `/`, `/home`, `$HOME`;
  - `/usr`, `/etc`, `/var`, `/boot`, `/opt`, `/srv`, `/root`, `/proc`, `/sys`, `/dev`, `/run`;
  - mount points.
- **Windows:**
  - drive roots;
  - `C:\Windows`, `C:\Program Files*`, `C:\ProgramData`, `C:\Users`, `%USERPROFILE%`;
  - `pagefile.sys`, `hiberfil.sys`, `swapfile.sys`;
  - reparse-point targets.
- **Re-validate immediately before acting (TOCTOU).**
  - `lstat(path)` must still be the same `(dev, ino, file type)` recorded during the scan.
  - It must not have become a symlink or a mount point.
  - Otherwise abort: "changed since scan; rescan".
- **Confirmation dialog contents:**
  - path, size on disk, item count;
  - a warning if the item is hard-linked elsewhere (`H`), shares clones, or is cloud-only;
  - for iCloud: "deletes from all devices".

### 9.4 Why the freed space may not show up
- **Hint text after deletion:**
  - Items in the Trash still use space; empty the Trash.
  - Local Time Machine snapshots keep the blocks until thinned (§1.10).
  - APFS clones: only private blocks are freed (§1.4).
  - Other hard links keep the data (§1.8).
  - Files held open by a running app are freed only when it closes them.
  - Cloud-only files freed ~0 locally.
  - Finder's "available" includes purgeable space (§1.9), so numbers differ between apps.
- **v1:**
  - Optimistically subtract the item's size from all ancestors.
  - Re-stat the parent.
  - Refresh free space with `statfs`.
  - If free space grew much less than expected, show the hint.

---

## 10. Traversal policy (pseudo-code)

```text
SCAN(root_input):
  # ---------- once per process ----------
  [macos] setiopolicy_np(TRIGGER_RESOLVE=5, PROCESS, OFF=1)
          setiopolicy_np(ATIME_UPDATES=2,  PROCESS, OFF=1)
  [unix]  raise RLIMIT_NOFILE to min(hard, kern.maxfilesperproc|/proc/sys/fs/nr_open, 10240)

  # ---------- per scan ----------
  root      := canonicalize(root_input)            # follow symlinks on the *root only* (du -H semantics)
  rmeta     := lstat(root)                          # must be a directory
  mounts    := snapshot()                           # macOS getmntinfo(MNT_NOWAIT); Linux /proc/self/mountinfo
  root_mnt  := mount containing root                # macOS statfs(root).f_mntonname; Linux statx mnt_id / longest prefix
  allowed_devs := { rmeta.st_dev }                  # macOS: also covers the whole System+Data volume group
  data_remainder := false
  F := {}                                           # firmlink targets (dev, ino)
  if macOS:
      firmlinks := parse("/usr/share/firmlinks")          # SOURCE \t TARGET (Data-relative)
      F := { (st_dev, st_ino) of stat(S) | S in firmlinks.sources, S exists }
      for each mount m: also register alias "/System/Volumes/Data/" + target(S) + rest  if m.on is under source S
      if root_mnt.flags has MNT_ROOTFS and root is an ancestor-or-self of "/System/Volumes":
          data_remainder := true                          # enter Data mount, but only for non-firmlinked content
  boundary := { m.on, aliases(m.on) for m in mounts if m != root_mnt and not (data_remainder and m is ROOT_DATA_VOL) }
  hard_excl := per-OS defaults (section 11) + user exclusions resolved to (dev, ino) where possible
  expected  := volume_usage(root) if root is a volume root (sum System+Data for "/") else None
  hardlinks := ShardedSet<(dev, ino)>()
  push WorkItem(dir=root, meta=rmeta, ancestors=[(rmeta.dev, rmeta.ino)], in_data_vol = (root under /System/Volumes/Data))

  # ---------- worker loop (N threads) ----------
  on thread start: [macos] setiopolicy_np(MATERIALIZE_DATALESS=3, THREAD, OFF=1)
  for each WorkItem w:
    node := tree.dir(w.dir); node.alloc += w.meta.blocks*512; node.apparent += w.meta.size
    [macos] if w.meta.st_flags & SF_DATALESS: node.state = CloudNotDownloaded; finish(node); continue
    entries := read_dir_fully(w.dir)               # then close handle; or getattrlistbulk on macOS
       on error e: node.state = classify(e)        # EACCES->Unreadable, EPERM->Blocked, EDEADLK->CloudNotDownloaded,
                                                   # ENOENT/ENOTDIR->Vanished(drop), ENAMETOOLONG->PathTooLong, EINTR->retry
                   finish(node); continue
    for name in entries:
      p := w.dir / name
      if p in hard_excl or (macOS and w.dir == "/" and name in {".vol", ".nofollow", ".resolve"}):
          add leaf Excluded(p); continue                     # no stat
      if p in boundary:
          add leaf OtherVolume(p, size = cached getmntinfo/SPACEUSED if local else unknown); continue   # no stat
      m := fstatat(dirfd, name, AT_SYMLINK_NOFOLLOW)  # std DirEntry::metadata()
         ENOENT/ENOTDIR -> stats.vanished++, continue;  EACCES/EPERM -> add error leaf, continue
      if m is symlink:   add leaf Symlink(alloc = m.blocks*512); continue          # never follow
      if m is dir:
          if m.dev not in allowed_devs:      add leaf OtherFilesystem(p); continue  # btrfs subvol, missed mounts, aliases
          if (m.dev, m.ino) in w.ancestors:  add leaf Loop(p); continue
          if w.in_data_vol and data_remainder and (m.dev, m.ino) in F:
                                             continue          # already counted via its firmlink path
          if [hfs volume] and ATTR_DIR_LINKCOUNT(p) > 1 and !visited_dirs.insert((m.dev,m.ino)): add leaf HardLinkDir; continue
          push WorkItem(p, m, w.ancestors + (m.dev, m.ino), in_data_vol = w.in_data_vol or p == "/System/Volumes/Data")
          continue
      # regular file / other
      if m.nlink > 1 and not hardlinks.insert((m.dev, m.ino)):
          add leaf HardLinkDuplicate(p, shown greyed, counted 0); continue
      add leaf File(p, alloc = m.blocks*512, apparent = m.size,
                    flags = [macos] compressed(UF_COMPRESSED) | dataless(SF_DATALESS) | hidden(UF_HIDDEN)
                            | sparse(alloc+4096 < size && !dataless))
    finish(node)    # when all children done: propagate totals + "incomplete" flags to ancestors; stream to UI

  # ---------- after ----------
  if expected: add synthetic "Not visible to this scan" = max(0, expected.bytes - root.alloc)
  report stats: items, bytes, errors by class, vanished, skipped volumes
```

Notes:
- The `boundary` check runs **before** any `stat`. That is the hang-safety property: dead network mounts and autofs triggers are never touched.
- The `st_dev` check is the backstop for mounts missed by path (renamed paths, mounts that appear mid-scan) and for btrfs subvolumes.
- On Linux, replace `m.dev not in allowed_devs` with `stx_mnt_id != w.mnt_id || m.dev not in allowed_devs` when `statx` is available. This catches bind mounts without a path lookup.
- Hard-link attribution is first-seen. Totals at the root are exact and deterministic.

---

## 11. Default exclusions and boundaries per OS

| OS | always skipped (boundary leaf, never stat'ed) | hard-excluded paths | opt-in includes |
|---|---|---|---|
| **macOS** | every mount point except the root's volume (and the Data volume in `data_remainder` mode): `/System/Volumes/{VM,Preboot,Update,Update/mnt1,Update/SFR/mnt1,xarts,iSCPreboot,Hardware}`, `/dev`, `/System/Volumes/Data/home` (autofs), `/Volumes/*` (external, DMG, network, Time Machine), FSKit/macFUSE mounts; plus Data-side aliases of all of these | `/.vol`, `/.nofollow`, `/.resolve` (at `/`); `Backups.backupdb`, `/Volumes/.timemachine`, `/Volumes/com.apple.TimeMachine.localsnapshots`; in "no FDA, avoid prompts" mode: `~/Desktop`, `~/Documents`, `~/Downloads`, `~/Library/Mobile Documents`, `~/Library/CloudStorage`, `~/Library/Containers`, `~/Library/Group Containers`, `~/.Trash` | per-leaf "scan this volume"; dataless cloud folders are never entered (no opt-in in v1) |
| **Linux** | every mount point except the root mount: proc, sysfs, devtmpfs, devpts, tmpfs, ramfs, cgroup(2), debugfs, tracefs, securityfs, pstore, bpf, configfs, mqueue, hugetlbfs, binfmt_misc, efivarfs, fusectl, nsfs, rpc_pipefs, autofs, squashfs (snaps), overlay, iso9660/udf, network/FUSE types (nfs, nfs4, cifs, smb3, 9p, ceph, glusterfs, lustre, afs, fuse.*), bind mounts; btrfs subvolumes (dev change) | `/proc`, `/sys`, `/dev`, `/run` (when root is `/`; same as the gdu and dua defaults); `.zfs` at dataset roots; `.snapshots` when subvolumes are opted in | other local disk filesystems (`/home`, `/boot`, data disks) and btrfs subvolumes, per leaf |
| **Windows** | all reparse points with name-surrogate tags (std `is_symlink()`: symlinks, junctions, mounted folders); other drive letters (separate roots); UNC shares unless chosen | none hard-coded; `System Volume Information` shows as unreadable | n/a |

User-facing setting: "Exclude caches marked with CACHEDIR.TAG" (off by default). Cargo writes `target/CACHEDIR.TAG` (M), the same as ncdu `--exclude-caches`. It is better used as a positive "regenerable cache – safe to delete" badge than as an exclusion.

---

## 12. Open questions and caveats

- macOS `/` itself was not walked (safety rules). The double count via `/System/Volumes/Data` and the 833 GB expectation are derived from the measured dev ids and volume stats, not from a full scan.
- iCloud dataless behaviour (`EDEADLK` vs. download despite the OFF policy) could not be tested here: no iCloud Drive, by design. The v1 design guards both ways: flag check plus policy plus error mapping.
- TCC prompt mechanics (blocking, per-launch App Data, purpose-string effects) are from Apple sessions and third-party reports. Verify them with a signed `.app` build before release.
- Probe timings were taken under load average ≈ 6.5 (parallel agents). Ratios are indicative only.

## Sources
- Apple SDK MacOSX27: `sys/attr.h`, `sys/stat.h`, `sys/mount.h`, `sys/resource.h`, `EndpointSecurity/ESClient.h`, `Foundation/NSURL.h`, `Foundation/NSFileManager.h`; man pages `getattrlist(2)`, `getattrlistbulk(2)`, `getiopolicy_np(3)`, `launchd.plist(5)`.
- xnu-12377.1.9 `bsd/vfs/vfs_syscalls.c` (dataless policy, `EDEADLK`), `vfs_attrlist.c` (pack order), `kpi_vfs.c`, `bsd/sys/proc_internal.h`. Apple file_cmds `du/du.c`, `df/df.c`.
- ncdu 2.9.2 (`ncdu.1`, `src/scan.zig`, `src/model.zig`); gdu (`pkg/device/*`, `pkg/analyze/*`); dua-cli (`src/options.rs`, `src/inodefilter.rs`, `crates/dua-lib/src/macos`); `trash` 5.2.9 (`src/macos/mod.rs`, `src/lib.rs`); `filesize` 0.2.0; Rust std 1.93 `sys/fs/{unix,windows}.rs`, `sys/path/windows.rs`.
- [Apple Platform Security – Protecting app access to user data](https://support.apple.com/guide/security/protecting-app-access-to-user-data-secddd1d86a6/web); [WWDC23 What's new in privacy](https://developer.apple.com/videos/play/wwdc2023/10053/); [WWDC24 What's new in privacy](https://developer.apple.com/videos/play/wwdc2024/10123/); [lapcatsoftware – Sonoma sandbox security](https://lapcatsoftware.com/articles/2023/6/1.html); [NSAppDataUsageDescription](https://developer.apple.com/documentation/bundleresources/information-property-list/nsappdatausagedescription); [Apple forums 709289 (Ventura deep links)](https://developer.apple.com/forums/thread/709289); [claude-code #85586 (FileProviderDomain prompts)](https://github.com/anthropics/claude-code/issues/85586); [claude-code #92784 (App Data session scope)](https://github.com/anthropics/claude-code/issues/92784); [Orca PR 9975 (usage strings)](https://github.com/stablyai/orca/pull/9975).
- [devclean PR #74 (dataless directories)](https://github.com/ohing504/devclean/pull/74); [Eclectic Light – Sonoma iCloud Drive](https://eclecticlight.co/2023/10/25/macos-sonoma-has-changed-icloud-drive-radically/).
- [man statx(2)](https://man7.org/linux/man-pages/man2/statx.2.html); [LWN 866709 – Btrfs inode numbers](https://lwn.net/Articles/866709/); [RH bugzilla 711881](https://bugzilla.redhat.com/show_bug.cgi?id=711881); [ArchWiki Btrfs](https://wiki.archlinux.org/title/Btrfs).
- [MS GetCompressedFileSizeW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getcompressedfilesizew).
- [DaisyDisk guide: hidden space](https://daisydiskapp.com/guide/4/en/HiddenSpace/), [purgeable](https://daisydiskapp.com/guide/4/en/PurgeableSpace/), [snapshots](https://daisydiskapp.com/guide/4/en/Snapshots/).
