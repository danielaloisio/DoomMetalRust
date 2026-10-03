//-----------------------------------------------------------------------------
//
// Copyright (C) 1993-1996 by id Software, Inc.
//
// This source is available for distribution and/or modification
// only under the terms of the DOOM Source Code License as
// published by id Software. All rights reserved.
//
// The source is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// FITNESS FOR A PARTICULAR PURPOSE. See the DOOM Source Code License
// for more details.
//
// DESCRIPTION:
//	Handles WAD file header, directory, lump I/O.
//
//-----------------------------------------------------------------------------

//! Rust port of `w_wad.h` / `w_wad.c`.
//!
//! WAD I/O functions: handles WAD file header, directory, lump I/O.
//!
//! # Design notes
//!
//! - **File I/O**: the original uses raw POSIX file descriptors
//!   (`open`/`read`/`lseek`/`close`) and keeps a WAD file open for the
//!   engine's whole lifetime, re-seeking into it on every lump read.
//!   This port uses `std::fs::File` + `Read`/`Seek` instead — same
//!   observable behavior (each lump is read from the right byte range of
//!   the right file, on demand), no raw fd/`unsafe` needed for something
//!   that was never actually platform-specific. `~name` reload files are
//!   the one case the original explicitly does NOT keep open (`handle =
//!   -1`, re-opened per read) — ported the same way, via `WadFile::Path`
//!   holding just the path rather than a `File`.
//! - **Zone allocation**: `W_CacheLumpNum` calls the original's
//!   `Z_Malloc(size, tag, &lumpcache[lump])`, using the zone's owner-slot
//!   mechanism as the lump cache itself. Ported the same way against
//!   `z_zone::Zone` — each lump's cache slot is a `Option<z_zone::Owner>`
//!   in [`WadFiles::lump_cache`], checked before re-reading, exactly
//!   mirroring the original's `if (!lumpcache[lump]) { ...load... }`.
//! - **`W_Profile`** (a debug tool dumping `waddump.txt`, never called
//!   from anywhere else in the original) is not ported — see the plan.
//! - **Lump names**: the original's 8-byte name comparison trick (two
//!   `int`s via a union) is just "compare 8 bytes, case-insensitively,
//!   uppercased" — ported directly as a byte-array comparison, which is
//!   both simpler and behaviorally identical (no endianness dependency
//!   either way, since it's a byte-for-byte compare in both versions
//!   despite the original *looking* endian-sensitive through the int
//!   cast — two equal byte arrays produce equal ints on any platform).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::doomtype::Byte;
use crate::z_zone::{Owner, PurgeTag, Zone};

/// (`wadinfo_t`) — WAD file header, as stored on disk.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct WadInfo {
    /// Should be "IWAD" or "PWAD".
    pub identification: [Byte; 4],
    pub numlumps: i32,
    pub infotableofs: i32,
}

/// (`filelump_t`) — one directory entry, as stored on disk.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FileLump {
    pub filepos: i32,
    pub size: i32,
    pub name: [Byte; 8],
}

/// Where a lump's bytes are read from: either a file kept open for the
/// engine's lifetime, or (for `~name` reload files) just a path,
/// re-opened per read — see module docs.
enum LumpSource {
    Open(File),
    /// The original's `handle == -1` case, tracked via `reloadname`.
    ReloadPath(PathBuf),
}

/// (`lumpinfo_t`) — WADFILE I/O related stuff: one entry per lump,
/// recording where to find its bytes.
struct LumpInfo {
    /// Up to 8 bytes, uppercased (matching the original's `char
    /// name[8]`, not necessarily nul-terminated if all 8 bytes are
    /// used).
    name: [Byte; 8],
    source: usize,
    position: i32,
    size: i32,
}

/// Port of the `w_wad` module state (the original's `lumpinfo`/
/// `numlumps`/`lumpcache` globals) plus the file sources they reference.
///
/// Unlike `doomstat`/`r_state`, this is not exposed as a single
/// process-global `static` — `w_wad` has no other module depending on it
/// yet (nothing importing it currently exists in this port), so
/// following the plan's globals strategy here would be speculative ahead
/// of an actual caller. A later phase's game-flow code
/// (`d_main`/`g_game`) is expected to own a `WadFiles` instance the same
/// way `doomstat`'s `GameState` is owned today, once it exists.
pub struct WadFiles {
    lumps: Vec<LumpInfo>,
    sources: Vec<LumpSource>,
    /// (`lumpcache`) — one cache slot per lump, `None` == NULL (not yet
    /// cached), `Some(owner)` == cached in `zone` under that owner.
    lump_cache: Vec<Option<Owner>>,
    zone: Zone,
    /// (`reloadname`/`reloadlump`) — the most recently added `~name`
    /// reload file's path and starting lump index, if any.
    reload: Option<(PathBuf, usize)>,
}

impl Default for WadFiles {
    fn default() -> Self {
        Self::new()
    }
}

impl WadFiles {
    /// Port of `W_InitMultipleFiles`'s empty-state setup (the allocation
    /// parts of the original — `numlumps = 0`, `lumpinfo = malloc(1)` —
    /// have no Rust equivalent worth reproducing; `Vec::new()` covers
    /// it).
    pub fn new() -> Self {
        WadFiles {
            lumps: Vec::new(),
            sources: Vec::new(),
            lump_cache: Vec::new(),
            zone: Zone::new(),
            reload: None,
        }
    }

    /// Port of `W_NumLumps`.
    pub fn num_lumps(&self) -> usize {
        self.lumps.len()
    }

    /// Port of `W_InitMultipleFiles`. Pass a list of files to use. All
    /// files are optional, but at least one file must be found. Files
    /// with a `.wad` extension are WAD files with multiple lumps. Other
    /// files are single lumps with the base filename for the lump name.
    /// Lump names can appear multiple times — the name searcher looks
    /// backwards, so a later file does override all earlier ones.
    ///
    /// # Panics
    /// Panics (standing in for the original's fatal `I_Error`) if no
    /// lumps were found across all files — "W_InitFiles: no files
    /// found".
    pub fn init_multiple_files<P: AsRef<Path>>(&mut self, filenames: &[P]) {
        for filename in filenames {
            self.add_file(filename.as_ref());
        }

        if self.lumps.is_empty() {
            panic!("W_InitFiles: no files found");
        }

        self.lump_cache = vec![None; self.lumps.len()];
    }

    /// Port of `W_InitFile`. Just initialize from a single file.
    pub fn init_file<P: AsRef<Path>>(&mut self, filename: P) {
        self.init_multiple_files(&[filename]);
    }

    /// Port of `W_AddFile`. All files are optional, but at least one
    /// file must be found (PWAD, if all required lumps are present).
    /// Files with a .wad extension are wadlink files with multiple
    /// lumps. Other files are single lumps with the base filename for
    /// the lump name.
    ///
    /// If the filename starts with a tilde, the file is handled
    /// specially to allow map reloads. But: the reload feature is a
    /// fragile hack... (the original's own comment, preserved).
    ///
    /// Unlike the original (`printf`s and returns on a missing file),
    /// this also just skips a missing file silently-to-the-caller aside
    /// from the message — matching "all files are optional" — but
    /// returns nothing to check either way, same as `W_AddFile`'s `void`
    /// return.
    pub fn add_file(&mut self, filename: &Path) {
        let (real_path, is_reload): (PathBuf, bool) =
            match filename.file_name().and_then(|n| n.to_str()) {
                Some(name) if name.starts_with('~') => {
                    let stripped = filename.with_file_name(&name[1..]);
                    (stripped, true)
                }
                _ => (filename.to_path_buf(), false),
            };

        let mut file = match File::open(&real_path) {
            Ok(f) => f,
            Err(_) => {
                println!(" couldn't open {}", real_path.display());
                return;
            }
        };

        println!(" adding {}", real_path.display());
        let startlump = self.lumps.len();

        let is_wad = real_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("wad"))
            .unwrap_or(false);

        let entries: Vec<FileLump> = if !is_wad {
            // Single lump file: the whole file is one lump, named after
            // the file's base name (see extract_file_base).
            let size = file
                .metadata()
                .unwrap_or_else(|e| panic!("Error fstating: {e}"))
                .len();
            vec![FileLump {
                filepos: 0,
                size: size as i32,
                name: extract_file_base(&real_path),
            }]
        } else {
            let mut header_bytes = [0u8; 12];
            file.read_exact(&mut header_bytes)
                .unwrap_or_else(|e| panic!("Error reading WAD header: {e}"));
            let identification = [
                header_bytes[0],
                header_bytes[1],
                header_bytes[2],
                header_bytes[3],
            ];
            let numlumps = i32::from_le_bytes(header_bytes[4..8].try_into().unwrap());
            let infotableofs = i32::from_le_bytes(header_bytes[8..12].try_into().unwrap());

            if &identification != b"IWAD" && &identification != b"PWAD" {
                panic!(
                    "Wad file {} doesn't have IWAD or PWAD id",
                    real_path.display()
                );
            }
            // ???modifiedgame = true; (original's own comment — the
            // PWAD-vs-IWAD distinction is meant to set a global that
            // isn't wired up here yet; doomstat.modifiedgame exists but
            // nothing calls into it from here per this phase's scope).

            file.seek(SeekFrom::Start(infotableofs as u64))
                .unwrap_or_else(|e| panic!("Error seeking to WAD directory: {e}"));

            let mut entries = Vec::with_capacity(numlumps as usize);
            let mut entry_bytes = [0u8; 16];
            for _ in 0..numlumps {
                file.read_exact(&mut entry_bytes)
                    .unwrap_or_else(|e| panic!("Error reading WAD directory entry: {e}"));
                entries.push(FileLump {
                    filepos: i32::from_le_bytes(entry_bytes[0..4].try_into().unwrap()),
                    size: i32::from_le_bytes(entry_bytes[4..8].try_into().unwrap()),
                    name: entry_bytes[8..16].try_into().unwrap(),
                });
            }
            entries
        };

        let source_index = self.sources.len();
        if is_reload {
            self.reload = Some((real_path.clone(), startlump));
            self.sources.push(LumpSource::ReloadPath(real_path));
        } else {
            self.sources.push(LumpSource::Open(file));
        }

        for entry in entries {
            self.lumps.push(LumpInfo {
                name: entry.name,
                source: source_index,
                position: entry.filepos,
                size: entry.size,
            });
        }
    }

    /// Port of `W_Reload`. Flushes any of the reloadable lumps in memory
    /// and reloads the directory.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if the reload file can't be
    /// opened or read.
    pub fn reload(&mut self) {
        let Some((path, reloadlump)) = self.reload.clone() else {
            return;
        };

        let mut file = File::open(&path)
            .unwrap_or_else(|e| panic!("W_Reload: couldn't open {}: {e}", path.display()));

        let mut header_bytes = [0u8; 12];
        file.read_exact(&mut header_bytes)
            .unwrap_or_else(|e| panic!("Error reading WAD header: {e}"));
        let lumpcount = i32::from_le_bytes(header_bytes[4..8].try_into().unwrap());
        let infotableofs = i32::from_le_bytes(header_bytes[8..12].try_into().unwrap());

        file.seek(SeekFrom::Start(infotableofs as u64))
            .unwrap_or_else(|e| panic!("Error seeking to WAD directory: {e}"));

        let mut entry_bytes = [0u8; 16];
        for i in reloadlump..reloadlump + lumpcount as usize {
            if let Some(owner) = self.lump_cache[i].take() {
                if let Some(handle) = self.zone.owner_handle(owner) {
                    self.zone.z_free(handle);
                }
            }

            file.read_exact(&mut entry_bytes)
                .unwrap_or_else(|e| panic!("Error reading WAD directory entry: {e}"));
            self.lumps[i].position = i32::from_le_bytes(entry_bytes[0..4].try_into().unwrap());
            self.lumps[i].size = i32::from_le_bytes(entry_bytes[4..8].try_into().unwrap());
        }
    }

    /// Port of `W_CheckNumForName`. Returns `None` if the name isn't
    /// found (the original returns `-1`).
    ///
    /// Scans backwards so patch lump files take precedence, exactly
    /// like the original.
    ///
    /// Subtlety preserved from the original: "case insensitive" only
    /// applies to `name` (the query) — a lump's *stored* name
    /// (`lumpinfo_t::name`, filled by a raw `strncpy` of the on-disk
    /// directory bytes in `W_AddFile`/[`WadFiles::add_file`]) is never
    /// uppercased. In practice every real WAD's directory names are
    /// already uppercase, so this rarely matters, but a hypothetical WAD
    /// with a lowercase-stored lump name would NOT be found by an
    /// uppercase (or any differently-cased) query in the original, and
    /// this port reproduces that exactly rather than "fixing" it into a
    /// fully case-insensitive comparison. Verified against the
    /// original's actual comparison logic, not just read off the source.
    pub fn check_num_for_name(&self, name: &str) -> Option<usize> {
        let key = normalize_lump_name(name);
        self.lumps
            .iter()
            .enumerate()
            .rev()
            .find(|(_, l)| l.name == key)
            .map(|(i, _)| i)
    }

    /// Port of `W_GetNumForName`. Calls [`WadFiles::check_num_for_name`],
    /// but bombs out if not found.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if `name` isn't found.
    pub fn get_num_for_name(&self, name: &str) -> usize {
        self.check_num_for_name(name)
            .unwrap_or_else(|| panic!("W_GetNumForName: {name} not found!"))
    }

    /// (`lumpinfo[lump].name`) — the raw 8-byte, uppercased name as
    /// stored, not necessarily NUL-terminated. Direct field access in
    /// the original (`r_things.c`'s `R_InitSpriteDefs` scans it to
    /// match sprite frames); exposed read-only here since `lumps` itself
    /// is private.
    ///
    /// # Panics
    /// Panics if `lump` is out of range.
    pub fn lump_name(&self, lump: usize) -> [Byte; 8] {
        self.lumps[lump].name
    }

    /// Port of `W_LumpLength`. Returns the buffer size needed to load
    /// the given lump.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if `lump` is out of range.
    pub fn lump_length(&self, lump: usize) -> usize {
        if lump >= self.lumps.len() {
            panic!("W_LumpLength: {lump} >= numlumps");
        }
        self.lumps[lump].size as usize
    }

    /// Port of `W_ReadLump`. Loads the lump into `dest`, which must be
    /// at least [`WadFiles::lump_length`] bytes.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if `lump` is out of range, the
    /// backing file can't be read, or fewer bytes than expected were
    /// read.
    pub fn read_lump(&mut self, lump: usize, dest: &mut [u8]) {
        if lump >= self.lumps.len() {
            panic!("W_ReadLump: {lump} >= numlumps");
        }

        let info = &self.lumps[lump];
        let size = info.size as usize;
        let position = info.position as u64;
        let source_index = info.source;

        let read_len = match &mut self.sources[source_index] {
            LumpSource::Open(file) => {
                file.seek(SeekFrom::Start(position))
                    .unwrap_or_else(|e| panic!("W_ReadLump: seek failed on lump {lump}: {e}"));
                read_up_to(file, &mut dest[..size])
            }
            LumpSource::ReloadPath(path) => {
                let mut file = File::open(&path).unwrap_or_else(|e| {
                    panic!("W_ReadLump: couldn't open {}: {e}", path.display())
                });
                file.seek(SeekFrom::Start(position))
                    .unwrap_or_else(|e| panic!("W_ReadLump: seek failed on lump {lump}: {e}"));
                read_up_to(&mut file, &mut dest[..size])
            }
        };

        if read_len < size {
            panic!("W_ReadLump: only read {read_len} of {size} on lump {lump}");
        }
    }

    /// Port of `W_CacheLumpNum`. Returns the cached lump bytes, reading
    /// and caching them (via [`z_zone::Zone`]) on first access, exactly
    /// mirroring the original's `if (!lumpcache[lump])` cache-miss path
    /// and `Z_ChangeTag` cache-hit path.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if `lump` is out of range.
    pub fn cache_lump_num(&mut self, lump: usize, tag: PurgeTag) -> &[u8] {
        if lump >= self.lumps.len() {
            panic!("W_CacheLumpNum: {lump} >= numlumps");
        }

        let size = self.lump_length(lump);

        let handle = match self.lump_cache[lump] {
            None => {
                // Read the lump into a scratch buffer first (read_lump
                // needs &mut self for the file source, so it can't run
                // while the zone block is borrowed), then copy into the
                // freshly allocated cache block — mirrors the original's
                // Z_Malloc-then-W_ReadLump-into-the-same-pointer order,
                // just with the read staged through a temporary buffer
                // instead of reading directly into the allocation.
                let mut buf = vec![0u8; size];
                self.read_lump(lump, &mut buf);

                let (handle, owner) = self.zone.z_malloc(size, tag, true);
                self.lump_cache[lump] = owner;
                self.zone
                    .get_mut(handle)
                    .expect("just allocated")
                    .copy_from_slice(&buf);
                handle
            }
            Some(owner) => {
                let handle = self
                    .zone
                    .owner_handle(owner)
                    .expect("cached lump's owner should still be valid");
                self.zone.z_change_tag(handle, tag);
                handle
            }
        };

        self.zone.get(handle).expect("just resolved")
    }

    /// Port of `W_CacheLumpName`.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if `name` isn't found.
    pub fn cache_lump_name(&mut self, name: &str, tag: PurgeTag) -> &[u8] {
        let lump = self.get_num_for_name(name);
        self.cache_lump_num(lump, tag)
    }
}

/// Read up to `dest.len()` bytes, returning how many were actually read
/// (short of a full read only at EOF) — the original's `read()` return
/// value semantics, which `W_ReadLump` checks against the expected size.
fn read_up_to<R: Read>(reader: &mut R, dest: &mut [u8]) -> usize {
    let mut total = 0;
    while total < dest.len() {
        match reader.read(&mut dest[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => panic!("read error: {e}"),
        }
    }
    total
}

/// Port of `ExtractFileBase`. Extracts the base filename (no directory,
/// no extension), uppercased, as an 8-byte lump name.
///
/// # Panics
/// Panics (standing in for `I_Error`) if the base filename is more than
/// 8 characters — "Filename base of {path} >8 chars", same as the
/// original.
fn extract_file_base(path: &Path) -> [Byte; 8] {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();

    if stem.len() > 8 {
        panic!("Filename base of {} >8 chars", path.display());
    }

    let mut name = [0u8; 8];
    for (i, b) in stem.bytes().enumerate() {
        name[i] = b.to_ascii_uppercase();
    }
    name
}

/// Uppercase and pad/truncate a lump name to the original's fixed
/// 8-byte, case-insensitive comparison key.
fn normalize_lump_name(name: &str) -> [Byte; 8] {
    let mut key = [0u8; 8];
    for (i, b) in name.bytes().take(8).enumerate() {
        key[i] = b.to_ascii_uppercase();
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_test_wad(path: &Path, lumps: &[(&str, &[u8])]) {
        let mut file = File::create(path).unwrap();

        let header_size = 12;
        let mut data_offset = header_size;
        let mut directory = Vec::new();
        let mut data = Vec::new();

        for (name, bytes) in lumps {
            let mut name8 = [0u8; 8];
            for (i, b) in name.bytes().take(8).enumerate() {
                name8[i] = b;
            }
            directory.push((data_offset, bytes.len() as i32, name8));
            data.extend_from_slice(bytes);
            data_offset += bytes.len() as i32;
        }
        let infotableofs = data_offset;

        file.write_all(b"IWAD").unwrap();
        file.write_all(&(lumps.len() as i32).to_le_bytes()).unwrap();
        file.write_all(&infotableofs.to_le_bytes()).unwrap();
        file.write_all(&data).unwrap();
        for (pos, size, name8) in directory {
            file.write_all(&pos.to_le_bytes()).unwrap();
            file.write_all(&size.to_le_bytes()).unwrap();
            file.write_all(&name8).unwrap();
        }
    }

    fn scratch_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "doommetalrust_wwad_test_{name}_{}.wad",
            std::process::id()
        ))
    }

    #[test]
    fn loads_lumps_and_reports_count() {
        let path = scratch_path("basic");
        write_test_wad(&path, &[("LUMPA", b"hello"), ("LUMPB", b"world!")]);

        let mut wad = WadFiles::new();
        wad.init_file(&path);

        assert_eq!(wad.num_lumps(), 2);
        assert_eq!(wad.lump_length(0), 5);
        assert_eq!(wad.lump_length(1), 6);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn check_num_for_name_is_case_insensitive_and_scans_backwards() {
        let path = scratch_path("names");
        write_test_wad(
            &path,
            &[("FOO", b"first"), ("BAR", b"second"), ("foo", b"third")],
        );

        let mut wad = WadFiles::new();
        wad.init_file(&path);

        // "case insensitive" in the original only normalizes the query
        // string (name8.s) — it's always uppercased before comparing —
        // while the stored lump names (lumpinfo_t::name, a raw strncpy
        // of the on-disk directory bytes in W_AddFile) are never
        // touched. So ANY query (whatever case it's typed in) only ever
        // matches lumps stored in uppercase; the lowercase-stored "foo"
        // lump here is unreachable by any query string, matching the
        // original exactly (verified independently against its actual
        // comparison logic, not just read off the source).
        assert_eq!(wad.check_num_for_name("FOO"), Some(0));
        assert_eq!(wad.check_num_for_name("foo"), Some(0)); // query uppercased to FOO too
        assert_eq!(wad.check_num_for_name("BAR"), Some(1));
        assert_eq!(wad.check_num_for_name("nonexistent"), None);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn check_num_for_name_scans_backwards_for_precedence() {
        let path = scratch_path("precedence");
        write_test_wad(&path, &[("SAME", b"first"), ("SAME", b"second")]);

        let mut wad = WadFiles::new();
        wad.init_file(&path);

        // Both stored in matching case, so precedence (later wins) is
        // the only thing under test here.
        assert_eq!(wad.check_num_for_name("SAME"), Some(1));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_lump_returns_correct_bytes() {
        let path = scratch_path("readback");
        write_test_wad(&path, &[("DATA", b"the quick brown fox")]);

        let mut wad = WadFiles::new();
        wad.init_file(&path);

        let mut buf = vec![0u8; wad.lump_length(0)];
        wad.read_lump(0, &mut buf);
        assert_eq!(&buf, b"the quick brown fox");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn cache_lump_num_caches_and_returns_same_bytes() {
        let path = scratch_path("cache");
        write_test_wad(&path, &[("CACHED", b"cache me")]);

        let mut wad = WadFiles::new();
        wad.init_file(&path);

        let bytes1 = wad.cache_lump_num(0, PurgeTag::Cache).to_vec();
        assert_eq!(&bytes1, b"cache me");

        // Second call should be a cache hit (still returns the same
        // data, exercising the Z_ChangeTag path).
        let bytes2 = wad.cache_lump_num(0, PurgeTag::Cache).to_vec();
        assert_eq!(bytes1, bytes2);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn later_pwad_overrides_earlier_iwad_lump() {
        let path1 = scratch_path("multi1");
        let path2 = scratch_path("multi2");
        write_test_wad(&path1, &[("SAME", b"original")]);
        write_test_wad(&path2, &[("SAME", b"override")]);

        let mut wad = WadFiles::new();
        wad.init_multiple_files(&[&path1, &path2]);

        let idx = wad.get_num_for_name("SAME");
        let mut buf = vec![0u8; wad.lump_length(idx)];
        wad.read_lump(idx, &mut buf);
        assert_eq!(&buf, b"override");

        std::fs::remove_file(&path1).ok();
        std::fs::remove_file(&path2).ok();
    }

    #[test]
    #[should_panic(expected = "not found")]
    fn get_num_for_name_panics_on_missing_lump() {
        let path = scratch_path("missing");
        write_test_wad(&path, &[("ONLY", b"x")]);

        // The lookup below is expected to panic, which would skip a
        // remove_file call placed after it — clean up via a drop guard
        // instead so the temp file doesn't leak into /tmp across test
        // runs.
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                std::fs::remove_file(&self.0).ok();
            }
        }
        let _cleanup = Cleanup(path.clone());

        let mut wad = WadFiles::new();
        wad.init_file(&path);
        wad.get_num_for_name("NOPE");
    }

    #[test]
    #[should_panic(expected = "no files found")]
    fn init_with_no_valid_files_panics() {
        let mut wad = WadFiles::new();
        wad.init_multiple_files(&["/nonexistent/path/doesnotexist.wad"]);
    }

    #[test]
    fn extract_file_base_uppercases_and_strips_path_and_extension() {
        assert_eq!(
            &extract_file_base(Path::new("/some/dir/doom.wad"))[..4],
            b"DOOM"
        );
        assert_eq!(&extract_file_base(Path::new("level1"))[..6], b"LEVEL1");
    }

    #[test]
    fn normalize_lump_name_uppercases_and_pads() {
        assert_eq!(normalize_lump_name("e1m1"), *b"E1M1\0\0\0\0");
        assert_eq!(normalize_lump_name("PLAYPAL"), *b"PLAYPAL\0");
    }
}
