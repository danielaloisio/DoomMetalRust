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
// Revision 1.3  1997/01/29 20:10
// DESCRIPTION:
//	Preparation of data for rendering,
//	generation of lookups, caching, retrieval by name.
//
//-----------------------------------------------------------------------------

//! Rust port of `r_data.h` / `r_data.c` (partial, see note below).
//! Refresh module, data I/O, caching, retrieval of graphics by name.
//!
//! # Scope
//!
//! Ported: [`RData::init`] (`R_InitData` — `R_InitTextures`,
//! `R_InitFlats`, `R_InitSpriteLumps`, `R_InitColormaps`),
//! [`RData::flat_num_for_name`] (`R_FlatNumForName`),
//! [`RData::texture_num_for_name`]/[`RData::check_texture_num_for_name`]
//! (`R_TextureNumForName`/`R_CheckTextureNumForName`), and the
//! composite-texture-cache pipeline ([`RData::get_column`] /
//! `R_GetColumn`, backed by `R_GenerateComposite`/
//! `R_DrawColumnInCache`/`R_GenerateLookup`, all folded into
//! [`RData::get_column`]'s private helpers since nothing else calls
//! them independently in the original either).
//!
//! Not ported: `R_PrecacheLevel` — depends on `numsprites`/`sprites`
//! (Phase 5e/7's sprite frame data) and `thinkercap`/`P_MobjThinker`
//! (Phase 6/7's live mobj pool), neither of which exist yet.
//!
//! # Patch parsing
//!
//! `patch_t` (ported in `r_defs::Patch`) is read directly off raw lump
//! bytes here — `columnofs` is `Vec<i32>` sized to `width` (see
//! `r_defs::Patch`'s doc comment on why, vs. the original's
//! over-allocated `columnofs[8]`).
//!
//! # Texture cache
//!
//! The original's composite-texture cache goes through the same global
//! zone (`mainzone`) as everything else via `Z_Malloc`/`W_CacheLumpNum`.
//! This port gives [`RData`] its own private `z_zone::Zone` instead of
//! sharing `WadFiles`'s internal one (which isn't exposed) — a
//! deliberate, harmless divergence given this port's zone allocator has
//! no real memory-pressure behavior to unify around (see `z_zone`'s
//! module docs); each single-patch column still resolves straight
//! through `WadFiles`'s own cache exactly as in the original, only the
//! *multi*-patch composite path uses `RData`'s separate zone.

use crate::doomtype::Byte;
use crate::m_fixed::{Fixed, FRACBITS};
use crate::r_defs::Patch;
use crate::w_wad::WadFiles;
use crate::z_zone::{Owner, PurgeTag, Zone};

/// A single patch placement within a composite texture (`mappatch_t`,
/// as read off `TEXTURE1`/`TEXTURE2` lump bytes).
#[derive(Debug, Clone, Copy)]
struct MapPatch {
    originx: i16,
    originy: i16,
    /// Index into the `PNAMES`-derived patch lookup table.
    patch: i16,
}

/// A patch placement resolved against `PNAMES`, ready for compositing
/// (`texpatch_t`).
#[derive(Debug, Clone, Copy)]
struct TexPatch {
    /// Block origin (always UL), which has already accounted for the
    /// internal origin of the patch (the original's own comment,
    /// preserved).
    originx: i32,
    originy: i32,
    /// Lump number of the patch.
    patch: usize,
}

/// A DOOM wall texture: a list of patches which are to be combined in a
/// predefined order (`texture_t`).
#[derive(Debug, Clone)]
struct Texture {
    /// Kept for switch changing, etc. (the original's own comment,
    /// preserved).
    name: [Byte; 8],
    width: i16,
    height: i16,
    /// All the `patches` are drawn back to front into the cached
    /// texture (the original's own comment, preserved).
    patches: Vec<TexPatch>,
}

/// Per-column lookup for one texture: either a direct single-patch lump
/// reference, or (once generated) an offset into the composited buffer.
#[derive(Debug, Clone, Copy)]
enum ColumnSource {
    /// (`texturecolumnlump[tex][col] >= 0`) — column has exactly one
    /// patch; `lump` is that patch's lump number, `ofs` the byte offset
    /// within it (already past the patch's 3-byte column header, per
    /// the original's `LONG(realpatch->columnofs[x-x1])+3`).
    SinglePatch { lump: usize, ofs: usize },
    /// (`texturecolumnlump[tex][col] == -1`) — column has multiple
    /// overlapping patches; `ofs` is the byte offset within the
    /// composited buffer once generated.
    Composite { ofs: usize },
}

/// Per-texture composite cache state.
#[derive(Debug, Default)]
struct CompositeCache {
    /// `None` until [`RData::generate_composite`] runs for this
    /// texture (`texturecomposite[tex] == 0` in the original).
    handle: Option<Owner>,
    size: usize,
}

/// Port of the `r_data` module state (the original's
/// `textures`/`texturecolumnlump`/`texturecolumnofs`/
/// `texturecomposite`/`texturecompositesize`/`texturewidthmask`/
/// `textureheight`/`texturetranslation`/`flattranslation`/`colormaps`/
/// `firstflat`/`lastflat`/`numflats`/`firstspritelump`/
/// `lastspritelump`/`numspritelumps` globals).
///
/// Like `w_wad::WadFiles`/`p_setup::Level`, this is plain-owned rather
/// than behind a `GlobalCell` — nothing outside this phase needs a
/// shared global instance yet.
#[derive(Debug, Default)]
pub struct RData {
    textures: Vec<Texture>,
    texture_columns: Vec<Vec<ColumnSource>>,
    texture_widthmask: Vec<i32>,
    pub textureheight: Vec<Fixed>,
    pub texturetranslation: Vec<i32>,
    composite_cache: Vec<CompositeCache>,
    zone: Zone,

    pub firstflat: i32,
    pub lastflat: i32,
    pub numflats: i32,
    pub flattranslation: Vec<i32>,

    pub firstspritelump: i32,
    pub lastspritelump: i32,
    pub numspritelumps: i32,
    pub spritewidth: Vec<Fixed>,
    pub spriteoffset: Vec<Fixed>,
    pub spritetopoffset: Vec<Fixed>,

    /// (`colormaps`) — 256-byte-aligned light tables in the original;
    /// alignment has no meaning for a `Vec<u8>` here, so just the raw
    /// bytes are kept (see [`RData::init_colormaps`]).
    pub colormaps: Vec<Byte>,
}

impl RData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `R_InitData`. Locates all the lumps that will be used by
    /// all views. Must be called after `W_Init` (the original's own
    /// comment — here, after `wad` is populated via
    /// [`WadFiles::init_file`]/[`WadFiles::init_multiple_files`]).
    pub fn init(&mut self, wad: &mut WadFiles) {
        self.init_textures(wad);
        self.init_flats(wad);
        self.init_sprite_lumps(wad);
        self.init_colormaps(wad);
    }

    /// Port of `R_InitTextures`. Initializes the texture list with the
    /// textures from the world map.
    ///
    /// The original's progress-bar `printf`s are not ported (no
    /// equivalent value in a non-interactive port; see `w_wad`'s "
    /// adding %s\n" precedent for the one case of original console
    /// output this port does keep, which is diagnostic rather than
    /// decorative).
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) on a bad texture directory
    /// offset or a texture referencing a missing patch — same fatal
    /// conditions as the original.
    fn init_textures(&mut self, wad: &mut WadFiles) {
        // Load the patch names from pnames.lmp.
        let pnames = wad.cache_lump_name("PNAMES", PurgeTag::Static).to_vec();
        let nummappatches = i32::from_le_bytes(pnames[0..4].try_into().unwrap()) as usize;
        let mut patchlookup = Vec::with_capacity(nummappatches);
        for i in 0..nummappatches {
            let off = 4 + i * 8;
            let name = lump_name_from_bytes(&pnames[off..off + 8]);
            patchlookup.push(wad.check_num_for_name(&name));
        }

        // Load the map texture definitions from textures.lmp. The data
        // is contained in one or two lumps, TEXTURE1 for shareware,
        // plus TEXTURE2 for commercial (the original's own comment,
        // preserved).
        let maptex1 = wad.cache_lump_name("TEXTURE1", PurgeTag::Static).to_vec();
        let numtextures1 = i32::from_le_bytes(maptex1[0..4].try_into().unwrap()) as usize;

        let maptex2 = if wad.check_num_for_name("TEXTURE2").is_some() {
            Some(wad.cache_lump_name("TEXTURE2", PurgeTag::Static).to_vec())
        } else {
            None
        };
        let numtextures2 = maptex2
            .as_ref()
            .map(|m| i32::from_le_bytes(m[0..4].try_into().unwrap()) as usize)
            .unwrap_or(0);

        let numtextures = numtextures1 + numtextures2;
        self.textures = Vec::with_capacity(numtextures);
        self.texture_widthmask = vec![0; numtextures];
        self.textureheight = vec![0; numtextures];

        for i in 0..numtextures {
            let (maptex, local_i) = if i < numtextures1 {
                (&maptex1, i)
            } else {
                (maptex2.as_ref().unwrap(), i - numtextures1)
            };

            let directory_off = 4 + local_i * 4;
            let offset =
                i32::from_le_bytes(maptex[directory_off..directory_off + 4].try_into().unwrap())
                    as usize;
            assert!(
                offset <= maptex.len(),
                "R_InitTextures: bad texture directory"
            );

            let mt = &maptex[offset..];
            let name = lump_name_from_bytes(&mt[0..8]);
            // mt[8..12] is `masked` (int), unused by this port same as
            // the original never reads it either (dead field even in
            // the C source — `masked` is written by the texture
            // compiler but nothing in r_data.c ever branches on it).
            let width = i16::from_le_bytes([mt[12], mt[13]]);
            let height = i16::from_le_bytes([mt[14], mt[15]]);
            // mt[16..20] is `obsolete_columndirectory` (int), unused.
            let patchcount = i16::from_le_bytes([mt[20], mt[21]]) as usize;

            let mut patches = Vec::with_capacity(patchcount);
            for j in 0..patchcount {
                let poff = offset + 22 + j * 10;
                let mp = MapPatch {
                    originx: i16::from_le_bytes([maptex[poff], maptex[poff + 1]]),
                    originy: i16::from_le_bytes([maptex[poff + 2], maptex[poff + 3]]),
                    patch: i16::from_le_bytes([maptex[poff + 4], maptex[poff + 5]]),
                };
                let resolved = patchlookup[mp.patch as usize]
                    .unwrap_or_else(|| panic!("R_InitTextures: Missing patch in texture {name}"));
                patches.push(TexPatch {
                    originx: mp.originx as i32,
                    originy: mp.originy as i32,
                    patch: resolved,
                });
            }

            self.texture_widthmask[i] = {
                let mut j = 1i32;
                while j * 2 <= width as i32 {
                    j <<= 1;
                }
                j - 1
            };
            self.textureheight[i] = (height as i32) << FRACBITS;

            self.textures.push(Texture {
                name: name_to_bytes(&name),
                width,
                height,
                patches,
            });
        }

        // Precalculate whatever possible.
        self.texture_columns = vec![Vec::new(); numtextures];
        self.composite_cache = (0..numtextures)
            .map(|_| CompositeCache::default())
            .collect();
        for i in 0..numtextures {
            self.generate_lookup(wad, i);
        }

        // Create translation table for global animation.
        self.texturetranslation = (0..numtextures as i32).collect();
    }

    /// Port of `R_GenerateLookup`.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if a texture's composited
    /// size would exceed 64K, same as the original.
    fn generate_lookup(&mut self, wad: &mut WadFiles, texnum: usize) {
        let texture = &self.textures[texnum];
        let width = texture.width as usize;
        let mut patchcount = vec![0u32; width];
        let mut columns = vec![ColumnSource::SinglePatch { lump: 0, ofs: 0 }; width];

        for patch in &texture.patches {
            let realpatch = read_patch(wad, patch.patch);
            let x1 = patch.originx;
            let x2 = x1 + realpatch.width as i32;

            let xstart = x1.max(0);
            let xend = x2.min(texture.width as i32);

            for x in xstart..xend {
                let x = x as usize;
                patchcount[x] += 1;
                let colofs = realpatch.columnofs[(x as i32 - x1) as usize] as usize + 3;
                columns[x] = ColumnSource::SinglePatch {
                    lump: patch.patch,
                    ofs: colofs,
                };
            }
        }

        let mut compositesize = 0usize;
        for x in 0..width {
            if patchcount[x] == 0 {
                // R_GenerateLookup: column without a patch (%s) — the
                // original prints this and returns early, leaving the
                // rest of `columns`/`compositesize` from this call
                // unfinished (a real quirk: any later columns just keep
                // whatever was set on a previous call, or the
                // just-computed single-patch values up to this point).
                // Ported as an eprintln + early return for the same
                // behavior, matching the original's own commented-out
                // `I_Error` (it deliberately downgraded this to
                // non-fatal at some point in the original's history).
                eprintln!(
                    "R_GenerateLookup: column without a patch ({})",
                    lump_name_from_bytes(&self.textures[texnum].name)
                );
                self.texture_columns[texnum] = columns;
                self.composite_cache[texnum].size = compositesize;
                return;
            }

            if patchcount[x] > 1 {
                // Use the cached block.
                let ofs = compositesize;
                if compositesize > 0x10000 - texture.height as usize {
                    panic!("R_GenerateLookup: texture {texnum} is >64k");
                }
                columns[x] = ColumnSource::Composite { ofs };
                compositesize += texture.height as usize;
            }
        }

        self.texture_columns[texnum] = columns;
        self.composite_cache[texnum].size = compositesize;
    }

    /// Port of `R_GenerateComposite`. Using the texture definition, the
    /// composite texture is created from the patches, and each column
    /// is cached (the original's own comment, preserved).
    fn generate_composite(&mut self, wad: &mut WadFiles, texnum: usize) {
        let texture = &self.textures[texnum];
        let size = self.composite_cache[texnum].size;

        let (handle, owner) = self.zone.z_malloc(size, PurgeTag::Static, true);
        self.composite_cache[texnum].handle = owner;

        let mut block = vec![0u8; size];

        for patch in texture.patches.clone() {
            let realpatch = read_patch(wad, patch.patch);
            let realpatch_bytes = {
                let len = wad.lump_length(patch.patch);
                let mut buf = vec![0u8; len];
                wad.read_lump(patch.patch, &mut buf);
                buf
            };

            let x1 = patch.originx;
            let x2 = x1 + realpatch.width as i32;
            let xstart = x1.max(0);
            let xend = x2.min(self.textures[texnum].width as i32);

            for x in xstart..xend {
                let xu = x as usize;
                let ColumnSource::Composite { ofs } = self.texture_columns[texnum][xu] else {
                    // Column does not have multiple patches? (the
                    // original's own comment, preserved) — skip.
                    continue;
                };

                let columnofs = realpatch.columnofs[(x - x1) as usize] as usize;
                draw_column_in_cache(
                    &realpatch_bytes,
                    columnofs,
                    &mut block,
                    ofs,
                    patch.originy,
                    self.textures[texnum].height as i32,
                );
            }
        }

        // Now that the texture has been built in column cache, it is
        // purgeable from zone memory (the original's own comment,
        // preserved).
        self.zone
            .get_mut(handle)
            .expect("just allocated")
            .copy_from_slice(&block);
        self.zone.z_change_tag(handle, PurgeTag::Cache);
    }

    /// Port of `R_GetColumn`. Retrieve column data for span blitting.
    pub fn get_column(&mut self, wad: &mut WadFiles, tex: usize, col: i32) -> Vec<u8> {
        let col = (col & self.texture_widthmask[tex]) as usize;
        let source = self.texture_columns[tex][col];

        match source {
            ColumnSource::SinglePatch { lump, ofs } => {
                let bytes = wad.cache_lump_num(lump, PurgeTag::Cache);
                bytes[ofs..].to_vec()
            }
            ColumnSource::Composite { ofs } => {
                if self.composite_cache[tex].handle.is_none() {
                    self.generate_composite(wad, tex);
                }
                let owner = self.composite_cache[tex].handle.unwrap();
                let handle = self
                    .zone
                    .owner_handle(owner)
                    .expect("composite was just ensured live");
                let data = self.zone.get(handle).expect("just resolved");
                data[ofs..].to_vec()
            }
        }
    }

    /// `(column_t*)((byte*)R_GetColumn(tex, col) - 3)`, the masked-mid
    /// texture fetch in `R_RenderMaskedSegRange`: the same column as
    /// [`RData::get_column`], but starting at its post header instead
    /// of its first pixel, so it can be walked post by post.
    ///
    /// For a single-patch column this is exactly the patch's own
    /// `column_t`. For a composite column the original steps 3 bytes
    /// back into the flat composite buffer and parses pixel data as
    /// post headers — the source of vanilla's garbled multi-patch
    /// masked textures ("Medusa effect"). That is reproduced as far as
    /// it stays inside the buffer; the composite's column 0 (which
    /// would read 3 bytes before the buffer, into the zone block header)
    /// returns an empty column instead.
    pub fn get_masked_column(&mut self, wad: &mut WadFiles, tex: usize, col: i32) -> Vec<u8> {
        let col = (col & self.texture_widthmask[tex]) as usize;
        match self.texture_columns[tex][col] {
            ColumnSource::SinglePatch { lump, ofs } => {
                wad.cache_lump_num(lump, PurgeTag::Cache)[ofs - 3..].to_vec()
            }
            ColumnSource::Composite { ofs } if ofs < 3 => vec![0xff],
            ColumnSource::Composite { ofs } => {
                if self.composite_cache[tex].handle.is_none() {
                    self.generate_composite(wad, tex);
                }
                let owner = self.composite_cache[tex].handle.unwrap();
                let handle = self
                    .zone
                    .owner_handle(owner)
                    .expect("composite was just ensured live");
                self.zone.get(handle).expect("just resolved")[ofs - 3..].to_vec()
            }
        }
    }

    /// Port of `R_InitFlats`.
    fn init_flats(&mut self, wad: &mut WadFiles) {
        self.firstflat = wad.get_num_for_name("F_START") as i32 + 1;
        self.lastflat = wad.get_num_for_name("F_END") as i32 - 1;
        self.numflats = self.lastflat - self.firstflat + 1;

        self.flattranslation = (0..self.numflats).collect();
    }

    /// Port of `R_InitSpriteLumps`. Finds the width and hoffset of all
    /// sprites in the wad, so the sprite does not need to be cached
    /// completely just for having the header info ready during
    /// rendering (the original's own comment, preserved).
    fn init_sprite_lumps(&mut self, wad: &mut WadFiles) {
        self.firstspritelump = wad.get_num_for_name("S_START") as i32 + 1;
        self.lastspritelump = wad.get_num_for_name("S_END") as i32 - 1;
        self.numspritelumps = self.lastspritelump - self.firstspritelump + 1;

        self.spritewidth = vec![0; self.numspritelumps as usize];
        self.spriteoffset = vec![0; self.numspritelumps as usize];
        self.spritetopoffset = vec![0; self.numspritelumps as usize];

        for i in 0..self.numspritelumps as usize {
            let lump = self.firstspritelump as usize + i;
            let patch = read_patch(wad, lump);
            self.spritewidth[i] = (patch.width as i32) << FRACBITS;
            self.spriteoffset[i] = (patch.leftoffset as i32) << FRACBITS;
            self.spritetopoffset[i] = (patch.topoffset as i32) << FRACBITS;
        }
    }

    /// Port of `R_InitColormaps`. Load in the light tables, 256 byte
    /// align tables (the original's own comment, preserved).
    ///
    /// The 256-byte alignment the original performs on the raw pointer
    /// has no meaning for a `Vec<u8>` (Rust's allocator already gives
    /// stronger-than-needed alignment for a byte buffer, and nothing
    /// downstream in this port does pointer arithmetic that depends on
    /// it) — just the lump's bytes are kept.
    fn init_colormaps(&mut self, wad: &mut WadFiles) {
        let lump = wad.get_num_for_name("COLORMAP");
        let length = wad.lump_length(lump);
        let mut buf = vec![0u8; length];
        wad.read_lump(lump, &mut buf);
        self.colormaps = buf;
    }

    /// Port of `R_FlatNumForName`. Retrieval, get a flat number for a
    /// flat name.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if `name` isn't found.
    pub fn flat_num_for_name(&self, wad: &WadFiles, name: &str) -> i32 {
        let i = wad
            .check_num_for_name(name)
            .unwrap_or_else(|| panic!("R_FlatNumForName: {name} not found"));
        i as i32 - self.firstflat
    }

    /// Port of `R_CheckTextureNumForName`. Check whether texture is
    /// available. Filter out NoTexture indicator (the original's own
    /// comment, preserved).
    pub fn check_texture_num_for_name(&self, name: &str) -> Option<i32> {
        // "NoTexture" marker.
        if name.starts_with('-') {
            return Some(0);
        }

        self.textures
            .iter()
            .position(|t| lump_name_from_bytes(&t.name).eq_ignore_ascii_case(name))
            .map(|i| i as i32)
    }

    /// Port of `R_TextureNumForName`. Calls
    /// [`RData::check_texture_num_for_name`], aborts with error message.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if `name` isn't found.
    pub fn texture_num_for_name(&self, name: &str) -> i32 {
        self.check_texture_num_for_name(name)
            .unwrap_or_else(|| panic!("R_TextureNumForName: {name} not found"))
    }
}

/// Read one patch's header (`patch_t`) plus its `columnofs` table off a
/// lump, sized to the patch's actual `width` (see `r_defs::Patch`'s doc
/// comment on why this differs from the original's fixed `columnofs[8]`
/// array).
fn read_patch(wad: &mut WadFiles, lump: usize) -> Patch {
    let len = wad.lump_length(lump);
    let mut buf = vec![0u8; len];
    wad.read_lump(lump, &mut buf);
    parse_patch(&buf)
}

fn parse_patch(data: &[u8]) -> Patch {
    let width = i16::from_le_bytes([data[0], data[1]]);
    let height = i16::from_le_bytes([data[2], data[3]]);
    let leftoffset = i16::from_le_bytes([data[4], data[5]]);
    let topoffset = i16::from_le_bytes([data[6], data[7]]);

    let mut columnofs = Vec::with_capacity(width as usize);
    for i in 0..width as usize {
        let off = 8 + i * 4;
        columnofs.push(i32::from_le_bytes(data[off..off + 4].try_into().unwrap()));
    }

    Patch {
        width,
        height,
        leftoffset,
        topoffset,
        columnofs,
    }
}

/// Port of `R_DrawColumnInCache`. Clip and draw a column from a patch
/// into a cached post.
///
/// `patch_bytes` is the source patch's full raw lump bytes;
/// `column_start` is the byte offset within it where the column (a
/// `column_t`/post chain) begins. `cache`/`cache_offset` is the
/// destination composite buffer and this column's offset within it.
///
/// # `post_t`'s on-disk layout has a padding byte on each side of its data
///
/// `post_t` is declared as `{ byte topdelta; byte length; }` — 2 bytes —
/// but the original reads a post's data starting 3 bytes past its start
/// (`source = (byte*)patch + 3`) and advances to the next post
/// `patch->length + 4` bytes later, not `+2`/`+length+2` as the bare
/// struct size would suggest. That's one padding byte before the data
/// and one after (5 bytes of overhead per post: 1 topdelta + 1 length +
/// 1 pad + data + 1 pad), a real quirk of the format, not a struct
/// alignment artifact — confirmed empirically here by walking a real
/// sprite lump's (`TROOA1`) column post chain rather than assumed from
/// the struct declaration alone (a `length=1` post was observed to span
/// exactly 5 bytes to the next post's `topdelta`).
fn draw_column_in_cache(
    patch_bytes: &[u8],
    column_start: usize,
    cache: &mut [u8],
    cache_offset: usize,
    originy: i32,
    cacheheight: i32,
) {
    let mut pos = column_start;
    loop {
        let topdelta = patch_bytes[pos];
        if topdelta == 0xff {
            break;
        }
        let length = patch_bytes[pos + 1] as i32;
        let source_start = pos + 3;

        let mut count = length;
        let mut position = originy + topdelta as i32;

        if position < 0 {
            count += position;
            position = 0;
        }
        if position + count > cacheheight {
            count = cacheheight - position;
        }

        if count > 0 {
            let dest_start = cache_offset + position as usize;
            let dest_end = dest_start + count as usize;
            let src_end = source_start + count as usize;
            cache[dest_start..dest_end].copy_from_slice(&patch_bytes[source_start..src_end]);
        }

        // patch = (column_t*)((byte*)patch + patch->length + 4)
        pos += length as usize + 4;
    }
}

/// Read an 8-byte fixed-width lump/texture name, trimming trailing NULs
/// — the common `strncpy(name, raw, 8); name[8] = 0;` pattern used
/// throughout the original for lump/patch/texture names.
pub(crate) fn lump_name_from_bytes(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Pack a name string back into an 8-byte, NUL-padded array — the
/// inverse of [`lump_name_from_bytes`].
fn name_to_bytes(name: &str) -> [Byte; 8] {
    let mut out = [0u8; 8];
    let bytes = name.as_bytes();
    let len = bytes.len().min(8);
    out[..len].copy_from_slice(&bytes[..len]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lump_name_from_bytes_trims_trailing_nulls() {
        assert_eq!(lump_name_from_bytes(b"E1M1\0\0\0\0"), "E1M1");
        assert_eq!(lump_name_from_bytes(b"PLAYPAL\0"), "PLAYPAL");
    }

    #[test]
    fn check_texture_num_for_name_notexture_marker() {
        let rdata = RData::new();
        assert_eq!(rdata.check_texture_num_for_name("-"), Some(0));
    }

    #[test]
    fn draw_column_in_cache_copies_a_simple_post() {
        // One post: topdelta(1) + length(1) + a leading "unused" pad
        // byte + 3 data bytes + a trailing "unused" pad byte — the
        // post_t format's actual on-disk layout has a byte of padding
        // on both sides of the data (empirically confirmed by walking a
        // real sprite lump's (TROOA1) post chain: a post with length=1
        // spans exactly 5 bytes to the next post, i.e. length+4, not
        // length+2 as the bare topdelta+length header would suggest).
        // Then the 0xff terminator post.
        let patch = vec![0u8, 3, 0xAA, 10, 20, 30, 0xAA, 0xff];

        let mut cache = vec![0u8; 10];
        draw_column_in_cache(&patch, 0, &mut cache, 2, 0, 10);

        assert_eq!(&cache[2..5], &[10, 20, 30]);
    }

    #[test]
    fn draw_column_in_cache_clips_negative_position() {
        // One post: topdelta=2, length=5, leading pad, 5 data bytes,
        // trailing pad, then the terminator. originy=-5 => position =
        // -5+2 = -3. See the post_t layout note above.
        let patch = vec![2u8, 5, 0xAA, 1, 2, 3, 4, 5, 0xAA, 0xff];
        let mut cache = vec![0u8; 10];
        draw_column_in_cache(&patch, 0, &mut cache, 0, -5, 10);

        // count was 5, position was -3 -> count += position (5-3=2),
        // position clamped to 0. So 2 bytes copied, taken from the
        // start of `source` (source is never shifted by the position
        // clip in the original — only `count`/`position` change).
        assert_eq!(&cache[0..2], &[1, 2]);
    }
}
