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
//	Refresh of things, i.e. objects represented by sprites.
//
//-----------------------------------------------------------------------------

//! Rust port of `r_things.h` / `r_things.c`. Refresh of things,
//! i.e. objects represented by sprites (the original's own comment,
//! preserved).
//!
//! # Scope
//!
//! Everything in `r_things.c` is ported: `R_InitSprites`/
//! `R_InitSpriteDefs`/`R_InstallSpriteLump` ([`RThings::r_init_sprites`]),
//! `R_ClearSprites`, `R_NewVisSprite`, `R_DrawMaskedColumn`
//! ([`r_draw_masked_column`], also used by `r_segs.rs`'s masked mid
//! textures), `R_DrawVisSprite`, `R_ProjectSprite`, `R_AddSprites`,
//! `R_DrawPSprite`/`R_DrawPlayerSprites`, `R_SortVisSprites`,
//! `R_DrawSprite` and `R_DrawMasked`.
//!
//! # Stand-ins for not-yet-ported state
//!
//! - `sprnames[]` (`info.c`, Phase 7) is the `namelist` argument of
//!   [`RThings::r_init_sprites`], as in the original's
//!   `R_InitSprites(char** namelist)` signature.
//! - `sector_t.thinglist`/`mobj_t.snext` aren't threaded through the
//!   ported [`Mobj`]/[`Sector`] (see `r_defs.rs`'s module docs), so
//!   [`RThings::r_add_sprites`] takes the sector's things as an iterator
//!   supplied by the caller.
//! - `viewplayer` (`player_t`/`pspdef_t`/`state_t`, Phase 6/7) is reduced
//!   to the fields `R_DrawPlayerSprites` actually reads: [`PlayerSprites`].
//! - The view-size globals `R_ExecuteSetViewSize` computes (ported as
//!   `r_main.rs`'s `Renderer::r_execute_set_view_size`, which owns them) —
//!   `viewwidth`/`viewheight`/`detailshift`/`centerxfrac`/
//!   `centeryfrac`/`projection`/`pspritescale`/`pspriteiscale` — plus the
//!   per-frame `extralight`/`fixedcolormap`/`viewangleoffset` are
//!   bundled as [`ViewParams`].
//! - `colfunc`/`basecolfunc`/`fuzzcolfunc` (function pointers) become the
//!   [`ColFunc`] enum.
//!
//! # Representation choices
//!
//! `vissprites[MAXVISSPRITES]`/`vissprite_p` become a `Vec` capped at
//! [`MAXVISSPRITES`]; the original's `overflowsprite` (a scratch slot
//! that is filled in but never drawn) becomes "don't push".
//! `vsprsortedhead`'s linked list becomes a sorted index list — see
//! [`RThings::r_sort_vis_sprites`] for why a stable sort gives exactly
//! the original's order.
//!
//! `mfloorclip`/`mceilingclip` (pointers to whole clip arrays) are only
//! ever read at `[dc_x]` by `R_DrawMaskedColumn`, so
//! [`r_draw_masked_column`] takes the two clip values for its column
//! directly. `negonearray`/`screenheightarray` are built where needed
//! (`R_DrawPlayerSprites`) or represented by
//! [`crate::r_defs::SpriteClip`] (drawsegs).
//!
//! `spritelights` (a `lighttable_t**` row of `scalelight`) is an index
//! into [`RMain::scalelight`], like `r_segs.rs`'s `walllights`.
//!
//! # Deliberate divergence: low-detail `dc_x`
//!
//! Linuxdoom's `R_DrawColumnLow` doubles the global `dc_x` in place, and
//! `R_DrawMaskedColumn`/`R_DrawVisSprite` use that same global as their
//! loop variable, so in low detail a column with two posts draws its
//! second post at `4*x` and the sprite loop skips/overruns columns (the
//! DOS release used assembly column drawers without this problem). This
//! port sets [`RDraw::dc_x`] before every column draw instead, so low
//! detail renders as intended rather than reproducing the corruption.

use crate::doomdef::SCREENWIDTH;
use crate::m_fixed::{fixed_div, fixed_mul, Fixed, FRACBITS, FRACUNIT};
use crate::p_setup::Level;
use crate::r_data::{lump_name_from_bytes, RData};
use crate::r_defs::{
    mobj_flag, DrawSeg, Mobj, Sector, SpriteDef, SpriteFrame, VisSprite, SIL_BOTTOM, SIL_TOP,
};
use crate::r_draw::RDraw;
use crate::r_main::{RMain, LIGHTLEVELS, LIGHTSCALESHIFT, LIGHTSEGSHIFT, MAXLIGHTSCALE};
use crate::r_plane::RPlane;
use crate::r_segs::RSegs;
use crate::tables::ANG45;
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

/// (`MAXVISSPRITES`)
pub const MAXVISSPRITES: usize = 128;

/// (`MINZ`) — things closer than this to the view plane aren't drawn.
pub const MINZ: Fixed = FRACUNIT * 4;

/// (`BASEYCENTER`)
pub const BASEYCENTER: i32 = 100;

/// (`FF_FULLBRIGHT`, `p_pspr.h`) — frame flag: draw at full brightness.
pub const FF_FULLBRIGHT: i32 = 0x8000;
/// (`FF_FRAMEMASK`, `p_pspr.h`) — the frame number part of a frame.
pub const FF_FRAMEMASK: i32 = 0x7fff;

/// (`NUMPSPRITES`, `p_pspr.h`) — weapon and muzzle flash.
pub const NUMPSPRITES: usize = 2;

/// Highest frame letter + 1 a sprite can have (`sprtemp[29]`).
const MAXSPRITEFRAMES: usize = 29;

/// Stand-in for the view-size globals set by `R_ExecuteSetViewSize` and
/// the per-frame `extralight`/`fixedcolormap`/`viewangleoffset` — see
/// module docs.
#[derive(Debug, Clone, Copy)]
pub struct ViewParams {
    /// Already shifted by `detailshift`, as in the original.
    pub viewwidth: i32,
    pub viewheight: i32,
    /// 0 = high detail, 1 = low detail.
    pub detailshift: i32,
    pub centerxfrac: Fixed,
    pub centeryfrac: Fixed,
    pub projection: Fixed,
    pub pspritescale: Fixed,
    pub pspriteiscale: Fixed,
    pub extralight: i32,
    /// Index into [`RData::colormaps`] rows, `None` for no fixed map.
    pub fixedcolormap: Option<usize>,
    pub viewangleoffset: i32,
}

impl ViewParams {
    /// The derived-value part of `R_ExecuteSetViewSize` for a view of
    /// `viewwidth` (already `>> detailshift`) by `viewheight` pixels,
    /// with no extra light, no fixed colormap and no view angle offset.
    pub fn new(viewwidth: i32, viewheight: i32, detailshift: i32) -> Self {
        let centerxfrac = (viewwidth / 2) << FRACBITS;
        ViewParams {
            viewwidth,
            viewheight,
            detailshift,
            centerxfrac,
            centeryfrac: (viewheight / 2) << FRACBITS,
            projection: centerxfrac,
            pspritescale: FRACUNIT * viewwidth / SCREENWIDTH,
            pspriteiscale: FRACUNIT * SCREENWIDTH / viewwidth,
            extralight: 0,
            fixedcolormap: None,
            viewangleoffset: 0,
        }
    }
}

/// Which column drawer `colfunc` points at (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColFunc {
    /// `R_DrawColumn`
    Column,
    /// `R_DrawColumnLow`
    ColumnLow,
    /// `R_DrawFuzzColumn`
    Fuzz,
    /// `R_DrawTranslatedColumn`
    Translated,
}

impl ColFunc {
    /// (`basecolfunc`) — as chosen by `R_ExecuteSetViewSize`.
    pub fn base(detailshift: i32) -> Self {
        if detailshift == 0 {
            ColFunc::Column
        } else {
            ColFunc::ColumnLow
        }
    }

    fn draw(
        self,
        rdraw: &mut RDraw,
        screen: &mut [u8],
        colormaps: &[u8],
        source: &[u8],
        viewheight: i32,
    ) {
        match self {
            ColFunc::Column => rdraw.r_draw_column(screen, colormaps, source),
            ColFunc::ColumnLow => rdraw.r_draw_column_low(screen, colormaps, source),
            ColFunc::Fuzz => rdraw.r_draw_fuzz_column(screen, colormaps, viewheight),
            ColFunc::Translated => rdraw.r_draw_translated_column(screen, colormaps, source),
        }
    }
}

/// One player sprite (`pspdef_t` with its `state_t` resolved) — see
/// module docs.
#[derive(Debug, Clone, Copy)]
pub struct PSprite {
    /// `psp->state->sprite`
    pub sprite: i32,
    /// `psp->state->frame`, may include [`FF_FULLBRIGHT`].
    pub frame: i32,
    pub sx: Fixed,
    pub sy: Fixed,
}

/// The fields of `viewplayer` that `R_DrawPlayerSprites` reads — see
/// module docs.
#[derive(Debug, Clone, Copy)]
pub struct PlayerSprites {
    /// `viewplayer->mo->subsector->sector->lightlevel`
    pub sector_lightlevel: i32,
    /// `viewplayer->powers[pw_invisibility]`
    pub invisibility: i32,
    /// `viewplayer->psprites[]`; `None` where `psp->state == NULL`.
    pub psprites: [Option<PSprite>; NUMPSPRITES],
}

/// Everything `R_DrawMasked` reaches through globals in the original:
/// the level, texture/WAD data, the openings arena, the masked-seg
/// scratch state and the drawing target.
pub struct MaskedDrawContext<'a> {
    pub rmain: &'a RMain,
    pub rdata: &'a mut RData,
    pub wad: &'a mut WadFiles,
    pub level: &'a Level,
    pub rplane: &'a mut RPlane,
    pub rsegs: &'a mut RSegs,
    pub rdraw: &'a mut RDraw,
    pub screen: &'a mut [u8],
    pub view: &'a ViewParams,
}

/// Port of `R_DrawMaskedColumn`. Used for sprites and masked mid
/// textures. Masked means: partly transparent, i.e. stored in
/// posts/runs of opaque pixels (the original's own comment, preserved).
///
/// `column` starts at the column's first post header (a raw
/// `column_t`). `floorclip`/`ceilingclip` are `mfloorclip[dc_x]`/
/// `mceilingclip[dc_x]`. [`RDraw::dc_texturemid`], `dc_iscale` and
/// `dc_colormap` (and `dc_translation` for [`ColFunc::Translated`]) must
/// already be set, as in the original; `dc_texturemid` is restored
/// before returning.
///
/// Each post's pixels are handed to the column drawer as a 256-byte
/// copy starting at the post's data: the drawers read up to 127 bytes
/// past a post's start regardless of its length (only the post's own
/// bytes end up on screen), which the original satisfies by reading on
/// into the rest of the lump. The copy has the same bytes where the lump
/// has them and zeros past its end, instead of a slice index panic.
#[allow(clippy::too_many_arguments)]
pub fn r_draw_masked_column(
    rdraw: &mut RDraw,
    screen: &mut [u8],
    colormaps: &[u8],
    column: &[u8],
    dc_x: i32,
    spryscale: Fixed,
    sprtopscreen: Fixed,
    floorclip: i32,
    ceilingclip: i32,
    colfunc: ColFunc,
    viewheight: i32,
) {
    let basetexturemid = rdraw.dc_texturemid;

    let mut pos = 0;
    while pos + 1 < column.len() && column[pos] != 0xff {
        let topdelta = column[pos] as i32;
        let length = column[pos + 1] as i32;

        // calculate unclipped screen coordinates for post (the
        // original's own comment, preserved)
        let topscreen = sprtopscreen.wrapping_add(spryscale.wrapping_mul(topdelta));
        let bottomscreen = topscreen.wrapping_add(spryscale.wrapping_mul(length));

        let mut dc_yl = topscreen.wrapping_add(FRACUNIT - 1) >> FRACBITS;
        let mut dc_yh = bottomscreen.wrapping_sub(1) >> FRACBITS;

        if dc_yh >= floorclip {
            dc_yh = floorclip - 1;
        }
        if dc_yl <= ceilingclip {
            dc_yl = ceilingclip + 1;
        }

        if dc_yl <= dc_yh {
            let data = column.get(pos + 3..).unwrap_or(&[]);
            let mut source = [0u8; 256];
            let n = data.len().min(source.len());
            source[..n].copy_from_slice(&data[..n]);

            rdraw.dc_x = dc_x;
            rdraw.dc_yl = dc_yl;
            rdraw.dc_yh = dc_yh;
            rdraw.dc_texturemid = basetexturemid - (topdelta << FRACBITS);

            // Drawn by either R_DrawColumn or (SHADOW)
            // R_DrawFuzzColumn (the original's own comment, preserved).
            colfunc.draw(rdraw, screen, colormaps, &source, viewheight);
        }

        pos += length as usize + 4;
    }

    rdraw.dc_texturemid = basetexturemid;
}

/// `sprtemp[]` entry during `R_InitSpriteDefs`: `rotate` is the
/// original's tri-state `boolean` (`-1` = no lump seen yet, from the
/// `memset(sprtemp, -1, ...)`).
#[derive(Debug, Clone, Copy)]
struct SprTemp {
    rotate: i32,
    lump: [i16; 8],
    flip: [u8; 8],
}

const SPRTEMP_UNSET: SprTemp = SprTemp {
    rotate: -1,
    lump: [-1; 8],
    flip: [0xff; 8],
};

/// Port of `R_InstallSpriteLump`. `lump` is already relative to
/// `firstspritelump`. Returns nothing; updates `sprtemp`/`maxframe`
/// like the original's globals.
///
/// # Panics
/// Panics (standing in for `I_Error`) on the same malformed-name
/// conditions as the original.
fn install_sprite_lump(
    sprtemp: &mut [SprTemp; MAXSPRITEFRAMES],
    maxframe: &mut i32,
    spritename: &str,
    lump: i32,
    frame: u32,
    rotation: u32,
    flipped: bool,
) {
    if frame >= MAXSPRITEFRAMES as u32 || rotation > 8 {
        panic!("R_InstallSpriteLump: Bad frame characters in lump {lump}");
    }

    if frame as i32 > *maxframe {
        *maxframe = frame as i32;
    }

    let frame_char = (b'A' + frame as u8) as char;
    let temp = &mut sprtemp[frame as usize];

    if rotation == 0 {
        // the lump should be used for all rotations (the original's own
        // comment, preserved)
        if temp.rotate == 0 {
            panic!("R_InitSprites: Sprite {spritename} frame {frame_char} has multip rot=0 lump");
        }
        if temp.rotate == 1 {
            panic!(
                "R_InitSprites: Sprite {spritename} frame {frame_char} has rotations and a rot=0 lump"
            );
        }

        temp.rotate = 0;
        temp.lump = [lump as i16; 8];
        temp.flip = [flipped as u8; 8];
        return;
    }

    // the lump is only used for one rotation (the original's own
    // comment, preserved)
    if temp.rotate == 0 {
        panic!(
            "R_InitSprites: Sprite {spritename} frame {frame_char} has rotations and a rot=0 lump"
        );
    }

    temp.rotate = 1;

    // make 0 based (the original's own comment, preserved)
    let rotation = (rotation - 1) as usize;
    if temp.lump[rotation] != -1 {
        panic!(
            "R_InitSprites: Sprite {spritename} : {frame_char} : {} has two lumps mapped to it",
            (b'1' + rotation as u8) as char
        );
    }

    temp.lump[rotation] = lump as i16;
    temp.flip[rotation] = flipped as u8;
}

/// Port of the `r_things` module state: `sprites`/`numsprites`,
/// `vissprites`/`vissprite_p` and `spritelights`. See module docs.
#[derive(Debug, Default)]
pub struct RThings {
    /// (`sprites`/`numsprites`) — one entry per `namelist` name, with an
    /// empty `spriteframes` for names that have no lumps in the WAD.
    pub sprites: Vec<SpriteDef>,
    /// (`vissprites`/`vissprite_p`)
    pub vissprites: Vec<VisSprite>,
    /// (`spritelights`) — row index into [`RMain::scalelight`].
    pub spritelights: usize,
}

impl RThings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `R_InitSprites`/`R_InitSpriteDefs`. Called at program
    /// start (the original's own comment, preserved).
    ///
    /// Builds the sprite rotation matrixes to account for horizontally
    /// flipped sprites. Sprite lump names are 4 characters for the
    /// actor, a letter for the frame, and a number for the rotation. A
    /// sprite that is flippable will have an additional letter/number
    /// appended. The rotation character can be 0 to signify no
    /// rotations (the original's own comments, preserved).
    ///
    /// `modifiedgame` is `doomstat`'s flag: with PWADs loaded, the first
    /// frame of each lump is looked up by name so a PWAD's replacement
    /// wins.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if a sprite's lumps are
    /// inconsistent (missing frames/rotations, conflicting lumps).
    pub fn r_init_sprites(
        &mut self,
        wad: &WadFiles,
        rdata: &RData,
        namelist: &[&str],
        modifiedgame: bool,
    ) {
        self.sprites = Vec::with_capacity(namelist.len());

        let start = rdata.firstspritelump - 1;
        let end = rdata.lastspritelump + 1;

        // scan all the lump names for each of the names, noting the
        // highest frame letter (the original's own comment, preserved)
        for &spritename in namelist {
            let mut sprtemp = [SPRTEMP_UNSET; MAXSPRITEFRAMES];
            let mut maxframe = -1;
            let intname = &spritename.as_bytes()[..4];

            // scan the lumps, filling in the frames for whatever is found
            // (the original's own comment, preserved)
            for l in (start + 1)..end {
                let name = wad.lump_name(l as usize);
                if &name[..4] != intname {
                    continue;
                }

                let frame = (name[4] as i32 - b'A' as i32) as u32;
                let rotation = (name[5] as i32 - b'0' as i32) as u32;

                let patched = if modifiedgame {
                    wad.get_num_for_name(&lump_name_from_bytes(&name)) as i32
                } else {
                    l
                };

                install_sprite_lump(
                    &mut sprtemp,
                    &mut maxframe,
                    spritename,
                    patched - rdata.firstspritelump,
                    frame,
                    rotation,
                    false,
                );

                if name[6] != 0 {
                    let frame = (name[6] as i32 - b'A' as i32) as u32;
                    let rotation = (name[7] as i32 - b'0' as i32) as u32;
                    install_sprite_lump(
                        &mut sprtemp,
                        &mut maxframe,
                        spritename,
                        l - rdata.firstspritelump,
                        frame,
                        rotation,
                        true,
                    );
                }
            }

            // check the frames that were found for completeness (the
            // original's own comment, preserved)
            if maxframe == -1 {
                self.sprites.push(SpriteDef::default());
                continue;
            }

            maxframe += 1;

            for (frame, temp) in sprtemp.iter().enumerate().take(maxframe as usize) {
                let frame_char = (b'A' + frame as u8) as char;
                match temp.rotate {
                    // no rotations were found for that frame at all
                    -1 => panic!(
                        "R_InitSprites: No patches found for {spritename} frame {frame_char}"
                    ),
                    // only the first rotation is needed
                    0 => {}
                    // must have all 8 frames
                    _ => {
                        if temp.lump.contains(&-1) {
                            panic!(
                                "R_InitSprites: Sprite {spritename} frame {frame_char} is missing rotations"
                            );
                        }
                    }
                }
            }

            // allocate space for the frames present and copy sprtemp to
            // it (the original's own comment, preserved)
            let spriteframes = sprtemp[..maxframe as usize]
                .iter()
                .map(|t| SpriteFrame {
                    rotate: t.rotate != 0,
                    lump: t.lump,
                    flip: t.flip,
                })
                .collect();
            self.sprites.push(SpriteDef { spriteframes });
        }
    }

    /// Port of `R_ClearSprites`. Called at frame start (the original's
    /// own comment, preserved).
    pub fn r_clear_sprites(&mut self) {
        self.vissprites.clear();
    }

    /// Port of `R_NewVisSprite`: keeps `vis` unless [`MAXVISSPRITES`]
    /// are already queued (the original's `overflowsprite`).
    fn new_vis_sprite(&mut self, vis: VisSprite) {
        if self.vissprites.len() < MAXVISSPRITES {
            self.vissprites.push(vis);
        }
    }

    /// `&sprites[sprite].spriteframes[frame & FF_FRAMEMASK]`, with the
    /// original's `#ifdef RANGECHECK` checks.
    fn sprite_frame(&self, sprite: i32, frame: i32) -> &SpriteFrame {
        if sprite as u32 >= self.sprites.len() as u32 {
            panic!("R_ProjectSprite: invalid sprite number {sprite} ");
        }
        let sprdef = &self.sprites[sprite as usize];
        if (frame & FF_FRAMEMASK) as usize >= sprdef.spriteframes.len() {
            panic!("R_ProjectSprite: invalid sprite frame {sprite} : {frame} ");
        }
        &sprdef.spriteframes[(frame & FF_FRAMEMASK) as usize]
    }

    /// Port of `R_ProjectSprite`. Generates a vissprite for a thing if
    /// it might be visible (the original's own comment, preserved).
    pub fn r_project_sprite(
        &mut self,
        thing: &Mobj,
        rmain: &RMain,
        rdata: &RData,
        view: &ViewParams,
    ) {
        // transform the origin point (the original's own comment,
        // preserved)
        let tr_x = thing.x.wrapping_sub(rmain.viewx);
        let tr_y = thing.y.wrapping_sub(rmain.viewy);

        let gxt = fixed_mul(tr_x, rmain.viewcos);
        let gyt = fixed_mul(tr_y, rmain.viewsin).wrapping_neg();

        let tz = gxt.wrapping_sub(gyt);

        // thing is behind view plane? (the original's own comment,
        // preserved)
        if tz < MINZ {
            return;
        }

        let xscale = fixed_div(view.projection, tz);

        let gxt = fixed_mul(tr_x, rmain.viewsin).wrapping_neg();
        let gyt = fixed_mul(tr_y, rmain.viewcos);
        let mut tx = gyt.wrapping_add(gxt).wrapping_neg();

        // too far off the side? (the original's own comment, preserved)
        if tx.wrapping_abs() > (tz << 2) {
            return;
        }

        // decide which patch to use for sprite relative to player (the
        // original's own comment, preserved)
        let sprframe = self.sprite_frame(thing.sprite as i32, thing.frame);

        let (lump, flip) = if sprframe.rotate {
            // choose a different rotation based on player view (the
            // original's own comment, preserved)
            let ang = rmain.point_to_angle(thing.x, thing.y);
            let rot = (ang
                .wrapping_sub(thing.angle)
                .wrapping_add((ANG45 / 2).wrapping_mul(9))
                >> 29) as usize;
            (sprframe.lump[rot] as usize, sprframe.flip[rot] != 0)
        } else {
            // use single rotation for all views (the original's own
            // comment, preserved)
            (sprframe.lump[0] as usize, sprframe.flip[0] != 0)
        };

        // calculate edges of the shape (the original's own comment,
        // preserved)
        tx = tx.wrapping_sub(rdata.spriteoffset[lump]);
        let x1 = view.centerxfrac.wrapping_add(fixed_mul(tx, xscale)) >> FRACBITS;

        // off the right side? (the original's own comment, preserved)
        if x1 > view.viewwidth {
            return;
        }

        tx = tx.wrapping_add(rdata.spritewidth[lump]);
        let x2 = (view.centerxfrac.wrapping_add(fixed_mul(tx, xscale)) >> FRACBITS) - 1;

        // off the left side (the original's own comment, preserved)
        if x2 < 0 {
            return;
        }

        // store information in a vissprite (the original's own comment,
        // preserved)
        let gzt = thing.z + rdata.spritetopoffset[lump];
        let iscale = fixed_div(FRACUNIT, xscale);
        let mut vis = VisSprite {
            x1: x1.max(0),
            x2: if x2 >= view.viewwidth {
                view.viewwidth - 1
            } else {
                x2
            },
            gx: thing.x,
            gy: thing.y,
            gz: thing.z,
            gzt,
            startfrac: 0,
            scale: xscale << view.detailshift,
            xiscale: iscale,
            texturemid: gzt - rmain.viewz,
            patch: lump as i32,
            colormap: None,
            mobjflags: thing.flags,
        };

        if flip {
            vis.startfrac = rdata.spritewidth[lump] - 1;
            vis.xiscale = -iscale;
        }

        if vis.x1 > x1 {
            vis.startfrac = vis
                .startfrac
                .wrapping_add(vis.xiscale.wrapping_mul(vis.x1 - x1));
        }

        // get light level (the original's own comment, preserved)
        vis.colormap = if thing.flags & mobj_flag::SHADOW != 0 {
            // shadow draw
            None
        } else if view.fixedcolormap.is_some() {
            // fixed map
            view.fixedcolormap
        } else if thing.frame & FF_FULLBRIGHT != 0 {
            // full bright
            Some(0)
        } else {
            // diminished light
            let index =
                ((xscale >> (LIGHTSCALESHIFT - view.detailshift)) as usize).min(MAXLIGHTSCALE - 1);
            Some(rmain.scalelight[self.spritelights][index] as usize)
        };

        self.new_vis_sprite(vis);
    }

    /// Port of `R_AddSprites`. During BSP traversal, this adds sprites
    /// by sector (the original's own comment, preserved).
    ///
    /// `things` stands in for walking `sec->thinglist` — see module docs.
    pub fn r_add_sprites<'m>(
        &mut self,
        sector: &mut Sector,
        things: impl IntoIterator<Item = &'m Mobj>,
        rmain: &RMain,
        rdata: &RData,
        view: &ViewParams,
    ) {
        // BSP is traversed by subsector. A sector might have been split
        // into several subsectors during BSP building. Thus we check
        // whether its already added (the original's own comment,
        // preserved).
        if sector.validcount == rmain.validcount {
            return;
        }

        // Well, now it will be done (the original's own comment,
        // preserved).
        sector.validcount = rmain.validcount;

        let lightnum = (sector.lightlevel as i32 >> LIGHTSEGSHIFT) + view.extralight;
        self.spritelights = light_row(lightnum);

        // Handle all things in sector (the original's own comment,
        // preserved).
        for thing in things {
            self.r_project_sprite(thing, rmain, rdata, view);
        }
    }

    /// Port of `R_DrawVisSprite`. `mfloorclip`/`mceilingclip` are whole
    /// screen-width arrays indexed by screen column. (The original's
    /// unused `x1`/`x2` parameters are dropped; it draws `vis->x1..=x2`.)
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) on a texture column outside
    /// the patch, same `#ifdef RANGECHECK` guard as the original.
    fn r_draw_vis_sprite(
        vis: &VisSprite,
        ctx: &mut MaskedDrawContext,
        mfloorclip: &[i32],
        mceilingclip: &[i32],
    ) {
        let lump = (vis.patch + ctx.rdata.firstspritelump) as usize;
        let patch = ctx.wad.cache_lump_num(lump, PurgeTag::Cache);
        let rdraw = &mut *ctx.rdraw;
        let view = ctx.view;

        let colfunc = match vis.colormap {
            // NULL colormap = shadow draw (the original's own comment,
            // preserved)
            None => ColFunc::Fuzz,
            Some(colormap) => {
                rdraw.dc_colormap = colormap;
                let translation = vis.mobjflags & mobj_flag::TRANSLATION;
                if translation != 0 {
                    // `translationtables - 256 + (flags & MF_TRANSLATION)
                    // >> (MF_TRANSSHIFT-8)`: translation value 1..=3
                    // selects row 0..=2.
                    rdraw.dc_translation =
                        Some((translation >> mobj_flag::TRANSSHIFT) as usize - 1);
                    ColFunc::Translated
                } else {
                    ColFunc::base(view.detailshift)
                }
            }
        };

        rdraw.dc_iscale = vis.xiscale.wrapping_abs() >> view.detailshift;
        rdraw.dc_texturemid = vis.texturemid;
        let mut frac = vis.startfrac;
        let spryscale = vis.scale;
        let sprtopscreen = view.centeryfrac - fixed_mul(rdraw.dc_texturemid, spryscale);

        let width = i16::from_le_bytes([patch[0], patch[1]]) as i32;
        for dc_x in vis.x1..=vis.x2 {
            let texturecolumn = frac >> FRACBITS;
            if texturecolumn < 0 || texturecolumn >= width {
                panic!("R_DrawSpriteRange: bad texturecolumn");
            }
            let ofs = 8 + 4 * texturecolumn as usize;
            let columnofs = i32::from_le_bytes(patch[ofs..ofs + 4].try_into().unwrap()) as usize;

            r_draw_masked_column(
                rdraw,
                ctx.screen,
                &ctx.rdata.colormaps,
                &patch[columnofs..],
                dc_x,
                spryscale,
                sprtopscreen,
                mfloorclip[dc_x as usize],
                mceilingclip[dc_x as usize],
                colfunc,
                view.viewheight,
            );
            frac = frac.wrapping_add(vis.xiscale);
        }
    }

    /// Port of `R_DrawPSprite`.
    fn r_draw_psprite(&self, psp: &PSprite, invisibility: i32, ctx: &mut MaskedDrawContext) {
        let view = *ctx.view;
        let rdata = &*ctx.rdata;

        // decide which patch to use (the original's own comment,
        // preserved)
        let sprframe = self.sprite_frame(psp.sprite, psp.frame);
        let lump = sprframe.lump[0] as usize;
        let flip = sprframe.flip[0] != 0;

        // calculate edges of the shape (the original's own comment,
        // preserved)
        let mut tx = psp.sx - 160 * FRACUNIT;

        tx -= rdata.spriteoffset[lump];
        let x1 = (view.centerxfrac + fixed_mul(tx, view.pspritescale)) >> FRACBITS;

        // off the right side (the original's own comment, preserved)
        if x1 > view.viewwidth {
            return;
        }

        tx += rdata.spritewidth[lump];
        let x2 = ((view.centerxfrac + fixed_mul(tx, view.pspritescale)) >> FRACBITS) - 1;

        // off the left side (the original's own comment, preserved)
        if x2 < 0 {
            return;
        }

        // store information in a vissprite (the original's own comment,
        // preserved)
        let mut vis = VisSprite {
            x1: x1.max(0),
            x2: if x2 >= view.viewwidth {
                view.viewwidth - 1
            } else {
                x2
            },
            gx: 0,
            gy: 0,
            gz: 0,
            gzt: 0,
            startfrac: 0,
            scale: view.pspritescale << view.detailshift,
            xiscale: view.pspriteiscale,
            texturemid: (BASEYCENTER << FRACBITS) + FRACUNIT / 2
                - (psp.sy - rdata.spritetopoffset[lump]),
            patch: lump as i32,
            colormap: None,
            mobjflags: 0,
        };

        if flip {
            vis.xiscale = -view.pspriteiscale;
            vis.startfrac = rdata.spritewidth[lump] - 1;
        }

        if vis.x1 > x1 {
            vis.startfrac += vis.xiscale * (vis.x1 - x1);
        }

        vis.colormap = if invisibility > 4 * 32 || invisibility & 8 != 0 {
            // shadow draw
            None
        } else if view.fixedcolormap.is_some() {
            // fixed color
            view.fixedcolormap
        } else if psp.frame & FF_FULLBRIGHT != 0 {
            // full bright
            Some(0)
        } else {
            // local light
            Some(ctx.rmain.scalelight[self.spritelights][MAXLIGHTSCALE - 1] as usize)
        };

        let screenheightarray = [view.viewheight; SCREENWIDTH as usize];
        let negonearray = [-1; SCREENWIDTH as usize];
        Self::r_draw_vis_sprite(&vis, ctx, &screenheightarray, &negonearray);
    }

    /// Port of `R_DrawPlayerSprites`.
    pub fn r_draw_player_sprites(&mut self, player: &PlayerSprites, ctx: &mut MaskedDrawContext) {
        // get light level (the original's own comment, preserved)
        let lightnum = (player.sector_lightlevel >> LIGHTSEGSHIFT) + ctx.view.extralight;
        self.spritelights = light_row(lightnum);

        // add all active psprites (the original's own comment,
        // preserved)
        for psp in player.psprites.iter().flatten() {
            self.r_draw_psprite(psp, player.invisibility, ctx);
        }
    }

    /// Port of `R_SortVisSprites`: indices into
    /// [`RThings::vissprites`], back (smallest scale) to front.
    ///
    /// The original repeatedly unlinks the first sprite with the
    /// strictly smallest scale from the remaining list, which is a
    /// stable ascending sort. (Its `bestscale = MAXINT` start would
    /// misbehave for a sprite with scale `MAXINT`, but a vissprite's
    /// scale is at most `projection / MINZ << detailshift`, far below
    /// that.)
    pub fn r_sort_vis_sprites(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.vissprites.len()).collect();
        order.sort_by_key(|&i| self.vissprites[i].scale);
        order
    }

    /// Port of `R_DrawSprite`: clips `spr` against every drawseg in
    /// front of it, drawing masked mid textures of drawsegs behind it
    /// first.
    ///
    /// # Panics
    /// Panics if a drawseg with a silhouette lacks the matching sprite
    /// clip array — `R_StoreWallRange` always saves one, and the
    /// original would read through `NULL` instead.
    pub fn r_draw_sprite(
        &self,
        spr: &VisSprite,
        drawsegs: &[DrawSeg],
        ctx: &mut MaskedDrawContext,
    ) {
        let mut clipbot = [-2i32; SCREENWIDTH as usize];
        let mut cliptop = [-2i32; SCREENWIDTH as usize];
        let viewheight = ctx.view.viewheight;

        // Scan drawsegs from end to start for obscuring segs. The first
        // drawseg that has a greater scale is the clip seg (the
        // original's own comment, preserved).
        for ds in drawsegs.iter().rev() {
            // determine if the drawseg obscures the sprite (the
            // original's own comment, preserved)
            if ds.x1 > spr.x2
                || ds.x2 < spr.x1
                || (ds.silhouette == 0 && ds.maskedtexturecol.is_none())
            {
                // does not cover sprite
                continue;
            }

            let r1 = ds.x1.max(spr.x1);
            let r2 = ds.x2.min(spr.x2);

            let (lowscale, scale) = if ds.scale1 > ds.scale2 {
                (ds.scale2, ds.scale1)
            } else {
                (ds.scale1, ds.scale2)
            };

            if scale < spr.scale
                || (lowscale < spr.scale
                    && RMain::point_on_seg_side(
                        spr.gx,
                        spr.gy,
                        &ctx.level.segs[ds.curline],
                        &ctx.level.vertexes,
                    ) == 0)
            {
                // masked mid texture? (the original's own comment,
                // preserved)
                if ds.maskedtexturecol.is_some() {
                    ctx.rsegs.r_render_masked_seg_range(
                        ctx.rmain, ctx.rdata, ctx.wad, ctx.level, ctx.rplane, ctx.rdraw,
                        ctx.screen, ctx.view, ds, r1, r2,
                    );
                }
                // seg is behind sprite
                continue;
            }

            // clip this piece of the sprite (the original's own comment,
            // preserved)
            let mut silhouette = ds.silhouette;

            if spr.gz >= ds.bsilheight {
                silhouette &= !SIL_BOTTOM;
            }

            if spr.gzt <= ds.tsilheight {
                silhouette &= !SIL_TOP;
            }

            let openings = &ctx.rplane.openings;
            let bottom = || {
                ds.sprbottomclip
                    .expect("R_DrawSprite: drawseg has no sprbottomclip")
            };
            let top = || {
                ds.sprtopclip
                    .expect("R_DrawSprite: drawseg has no sprtopclip")
            };

            for x in r1..=r2 {
                let xi = x as usize;
                // 1 = bottom sil, 2 = top sil, 3 = both
                if silhouette & SIL_BOTTOM != 0 && clipbot[xi] == -2 {
                    clipbot[xi] = bottom().at(x, ds.x1, openings, viewheight);
                }
                if silhouette & SIL_TOP != 0 && cliptop[xi] == -2 {
                    cliptop[xi] = top().at(x, ds.x1, openings, viewheight);
                }
            }
        }

        // all clipping has been performed, so draw the sprite (the
        // original's own comment, preserved)

        // check for unclipped columns (the original's own comment,
        // preserved)
        for x in spr.x1..=spr.x2 {
            let xi = x as usize;
            if clipbot[xi] == -2 {
                clipbot[xi] = viewheight;
            }
            if cliptop[xi] == -2 {
                cliptop[xi] = -1;
            }
        }

        Self::r_draw_vis_sprite(spr, ctx, &clipbot, &cliptop);
    }

    /// Port of `R_DrawMasked`: sprites back to front, then the masked
    /// mid textures no sprite pass drew, then (unless this is a side
    /// view, `viewangleoffset != 0`) the player's weapon sprites.
    ///
    /// `player` is `None` where there's no view player to draw psprites
    /// for (the original always has one).
    pub fn r_draw_masked(
        &mut self,
        drawsegs: &[DrawSeg],
        player: Option<&PlayerSprites>,
        ctx: &mut MaskedDrawContext,
    ) {
        let order = self.r_sort_vis_sprites();

        // draw all vissprites back to front (the original's own
        // comment, preserved)
        for i in order {
            let spr = self.vissprites[i];
            self.r_draw_sprite(&spr, drawsegs, ctx);
        }

        // render any remaining masked mid textures (the original's own
        // comment, preserved)
        for ds in drawsegs.iter().rev() {
            if ds.maskedtexturecol.is_some() {
                ctx.rsegs.r_render_masked_seg_range(
                    ctx.rmain, ctx.rdata, ctx.wad, ctx.level, ctx.rplane, ctx.rdraw, ctx.screen,
                    ctx.view, ds, ds.x1, ds.x2,
                );
            }
        }

        // draw the psprites on top of everything but does not draw on
        // side views (the original's own comment, preserved)
        if ctx.view.viewangleoffset == 0 {
            if let Some(player) = player {
                self.r_draw_player_sprites(player, ctx);
            }
        }
    }
}

/// `scalelight[lightnum]` row selection with the original's clamping,
/// shared by `R_AddSprites` and `R_DrawPlayerSprites`.
fn light_row(lightnum: i32) -> usize {
    if lightnum < 0 {
        0
    } else if lightnum >= LIGHTLEVELS as i32 {
        LIGHTLEVELS - 1
    } else {
        lightnum as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doomdef::SCREENHEIGHT;

    /// A colormap buffer where row `r` maps every color `c` to
    /// `c + r` (wrapping), so tests can tell which row was used.
    fn test_colormaps() -> Vec<u8> {
        (0..34 * 256).map(|i| ((i % 256) + i / 256) as u8).collect()
    }

    fn test_rdraw() -> RDraw {
        let mut rdraw = RDraw::new();
        rdraw.r_init_buffer(SCREENWIDTH, SCREENHEIGHT);
        rdraw
    }

    fn pixel(screen: &[u8], x: i32, y: i32) -> u8 {
        screen[(y * SCREENWIDTH + x) as usize]
    }

    #[test]
    fn install_sprite_lump_rot0_fills_all_rotations() {
        let mut sprtemp = [SPRTEMP_UNSET; MAXSPRITEFRAMES];
        let mut maxframe = -1;
        install_sprite_lump(&mut sprtemp, &mut maxframe, "BAR1", 7, 1, 0, false);
        assert_eq!(maxframe, 1);
        assert_eq!(sprtemp[1].rotate, 0);
        assert_eq!(sprtemp[1].lump, [7; 8]);
        assert_eq!(sprtemp[1].flip, [0; 8]);
        assert_eq!(sprtemp[0].rotate, -1);
    }

    #[test]
    fn install_sprite_lump_rotations_and_flip() {
        let mut sprtemp = [SPRTEMP_UNSET; MAXSPRITEFRAMES];
        let mut maxframe = -1;
        // TROOA2A8: lump 3 is rotation 2 as-is and rotation 8 flipped.
        install_sprite_lump(&mut sprtemp, &mut maxframe, "TROO", 3, 0, 2, false);
        install_sprite_lump(&mut sprtemp, &mut maxframe, "TROO", 3, 0, 8, true);
        assert_eq!(sprtemp[0].rotate, 1);
        assert_eq!(sprtemp[0].lump[1], 3);
        assert_eq!(sprtemp[0].flip[1], 0);
        assert_eq!(sprtemp[0].lump[7], 3);
        assert_eq!(sprtemp[0].flip[7], 1);
        assert_eq!(sprtemp[0].lump[0], -1);
    }

    #[test]
    #[should_panic(expected = "has rotations and a rot=0 lump")]
    fn install_sprite_lump_rejects_rot0_after_rotations() {
        let mut sprtemp = [SPRTEMP_UNSET; MAXSPRITEFRAMES];
        let mut maxframe = -1;
        install_sprite_lump(&mut sprtemp, &mut maxframe, "TROO", 0, 0, 1, false);
        install_sprite_lump(&mut sprtemp, &mut maxframe, "TROO", 1, 0, 0, false);
    }

    #[test]
    #[should_panic(expected = "has two lumps mapped to it")]
    fn install_sprite_lump_rejects_duplicate_rotation() {
        let mut sprtemp = [SPRTEMP_UNSET; MAXSPRITEFRAMES];
        let mut maxframe = -1;
        install_sprite_lump(&mut sprtemp, &mut maxframe, "TROO", 0, 0, 1, false);
        install_sprite_lump(&mut sprtemp, &mut maxframe, "TROO", 1, 0, 1, false);
    }

    #[test]
    #[should_panic(expected = "Bad frame characters")]
    fn install_sprite_lump_rejects_bad_frame_letter() {
        let mut sprtemp = [SPRTEMP_UNSET; MAXSPRITEFRAMES];
        let mut maxframe = -1;
        // A frame letter before 'A' wraps to a huge unsigned value.
        install_sprite_lump(
            &mut sprtemp,
            &mut maxframe,
            "TROO",
            0,
            (b'@' as i32 - b'A' as i32) as u32,
            0,
            false,
        );
    }

    #[test]
    fn draw_masked_column_draws_posts_at_unit_scale() {
        let colormaps = test_colormaps();
        let mut rdraw = test_rdraw();
        let mut screen = vec![0u8; (SCREENWIDTH * SCREENHEIGHT) as usize];

        // Two posts: rows 2..=4 with colors 10,11,12 and rows 8..=9 with
        // colors 20,21 (5 bytes of overhead per post, see r_data.rs).
        let column = [2, 3, 0, 10, 11, 12, 0, 8, 2, 0, 20, 21, 0, 0xff];

        rdraw.dc_colormap = 1;
        rdraw.dc_iscale = FRACUNIT;
        rdraw.dc_texturemid = 100 << FRACBITS;
        // Texture row 0 at screen row 0: sprtopscreen = centery - texturemid.
        let sprtopscreen = 0;

        r_draw_masked_column(
            &mut rdraw,
            &mut screen,
            &colormaps,
            &column,
            5,
            FRACUNIT,
            sprtopscreen,
            SCREENHEIGHT,
            -1,
            ColFunc::Column,
            SCREENHEIGHT,
        );

        let col: Vec<u8> = (0..11).map(|y| pixel(&screen, 5, y)).collect();
        assert_eq!(col, [0, 0, 11, 12, 13, 0, 0, 0, 21, 22, 0]);
        assert_eq!(
            rdraw.dc_texturemid,
            100 << FRACBITS,
            "dc_texturemid must be restored"
        );
    }

    #[test]
    fn draw_masked_column_respects_clip_values() {
        let colormaps = test_colormaps();
        let mut rdraw = test_rdraw();
        let mut screen = vec![0u8; (SCREENWIDTH * SCREENHEIGHT) as usize];
        let column = [0, 6, 0, 1, 2, 3, 4, 5, 6, 0, 0xff];

        rdraw.dc_iscale = FRACUNIT;
        rdraw.dc_texturemid = 100 << FRACBITS;

        // ceilingclip 1 / floorclip 4: only rows 2..=3 may be drawn.
        r_draw_masked_column(
            &mut rdraw,
            &mut screen,
            &colormaps,
            &column,
            0,
            FRACUNIT,
            0,
            4,
            1,
            ColFunc::Column,
            SCREENHEIGHT,
        );

        let col: Vec<u8> = (0..6).map(|y| pixel(&screen, 0, y)).collect();
        assert_eq!(col, [0, 0, 3, 4, 0, 0]);
    }

    #[test]
    fn sort_vis_sprites_is_stable_ascending_by_scale() {
        let base = VisSprite {
            x1: 0,
            x2: 0,
            gx: 0,
            gy: 0,
            gz: 0,
            gzt: 0,
            startfrac: 0,
            scale: 0,
            xiscale: 0,
            texturemid: 0,
            patch: 0,
            colormap: None,
            mobjflags: 0,
        };
        let mut things = RThings::new();
        for scale in [30, 10, 20, 10] {
            things.vissprites.push(VisSprite { scale, ..base });
        }
        assert_eq!(things.r_sort_vis_sprites(), [1, 3, 2, 0]);
    }

    #[test]
    fn new_vis_sprite_drops_past_maxvissprites() {
        let mut things = RThings::new();
        let vis = VisSprite {
            x1: 0,
            x2: 0,
            gx: 0,
            gy: 0,
            gz: 0,
            gzt: 0,
            startfrac: 0,
            scale: 0,
            xiscale: 0,
            texturemid: 0,
            patch: 0,
            colormap: None,
            mobjflags: 0,
        };
        for _ in 0..MAXVISSPRITES + 5 {
            things.new_vis_sprite(vis);
        }
        assert_eq!(things.vissprites.len(), MAXVISSPRITES);
    }

    /// A one-frame, single-rotation sprite 64 units wide, centered on
    /// its origin (leftoffset 32), 50 units tall above it.
    fn one_sprite_setup() -> (RThings, RData) {
        let mut things = RThings::new();
        things.sprites.push(SpriteDef {
            spriteframes: vec![SpriteFrame {
                rotate: false,
                lump: [0; 8],
                flip: [0; 8],
            }],
        });
        let mut rdata = RData::new();
        rdata.spritewidth = vec![64 << FRACBITS];
        rdata.spriteoffset = vec![32 << FRACBITS];
        rdata.spritetopoffset = vec![50 << FRACBITS];
        (things, rdata)
    }

    fn thing_at(x: i32, y: i32) -> Mobj {
        Mobj {
            x: x << FRACBITS,
            y: y << FRACBITS,
            ..Mobj::blank(crate::info::MobjType::MtPlayer)
        }
    }

    /// Viewer at the origin looking along +x (viewangle 0).
    fn view_along_x() -> RMain {
        let mut rmain = RMain::new();
        rmain.viewcos = FRACUNIT;
        rmain.viewsin = 0;
        rmain
    }

    #[test]
    fn project_sprite_straight_ahead_is_centered() {
        let (mut things, rdata) = one_sprite_setup();
        let rmain = view_along_x();
        let view = ViewParams::new(320, 200, 0);

        // 160 units away: xscale = 160/160 = 1.0, so the 64-wide sprite
        // spans 64 columns centered on column 160.
        things.r_project_sprite(&thing_at(160, 0), &rmain, &rdata, &view);

        assert_eq!(things.vissprites.len(), 1);
        let vis = things.vissprites[0];
        assert_eq!(vis.x1, 128);
        assert_eq!(vis.x2, 191);
        assert_eq!(vis.scale, FRACUNIT);
        assert_eq!(vis.xiscale, FRACUNIT);
        assert_eq!(vis.startfrac, 0);
        assert_eq!(vis.texturemid, 50 << FRACBITS);
    }

    #[test]
    fn project_sprite_behind_or_too_close_is_rejected() {
        let (mut things, rdata) = one_sprite_setup();
        let rmain = view_along_x();
        let view = ViewParams::new(320, 200, 0);

        things.r_project_sprite(&thing_at(-50, 0), &rmain, &rdata, &view);
        things.r_project_sprite(&thing_at(3, 0), &rmain, &rdata, &view);
        // Far off to the side: |tx| > 4*tz.
        things.r_project_sprite(&thing_at(10, 100), &rmain, &rdata, &view);
        assert!(things.vissprites.is_empty());
    }

    #[test]
    fn project_sprite_clips_left_edge_and_advances_startfrac() {
        let (mut things, rdata) = one_sprite_setup();
        let rmain = view_along_x();
        let view = ViewParams::new(320, 200, 0);

        // 150 units to the left at distance 160: left edge would be at
        // column 160 - 150 - 32 = -22.
        things.r_project_sprite(&thing_at(160, 150), &rmain, &rdata, &view);

        let vis = things.vissprites[0];
        assert_eq!(vis.x1, 0);
        assert_eq!(vis.x2, 41);
        assert_eq!(vis.startfrac, 22 * FRACUNIT);
    }

    #[test]
    fn project_sprite_light_selection() {
        let (mut things, rdata) = one_sprite_setup();
        let mut rmain = view_along_x();
        rmain.scalelight[3][16] = 7;
        things.spritelights = 3;
        let mut view = ViewParams::new(320, 200, 0);

        // xscale 1.0 >> LIGHTSCALESHIFT = 16.
        things.r_project_sprite(&thing_at(160, 0), &rmain, &rdata, &view);
        assert_eq!(things.vissprites[0].colormap, Some(7));

        let mut bright = thing_at(160, 0);
        bright.frame = FF_FULLBRIGHT;
        things.r_project_sprite(&bright, &rmain, &rdata, &view);
        assert_eq!(things.vissprites[1].colormap, Some(0));

        let mut shadow = thing_at(160, 0);
        shadow.flags = mobj_flag::SHADOW;
        things.r_project_sprite(&shadow, &rmain, &rdata, &view);
        assert_eq!(things.vissprites[2].colormap, None);

        view.fixedcolormap = Some(32);
        things.r_project_sprite(&thing_at(160, 0), &rmain, &rdata, &view);
        assert_eq!(things.vissprites[3].colormap, Some(32));
    }

    #[test]
    fn add_sprites_visits_each_sector_once_per_validcount() {
        let (mut things, rdata) = one_sprite_setup();
        let rmain = view_along_x();
        let view = ViewParams::new(320, 200, 0);
        let mut sector = Sector {
            lightlevel: 255,
            ..Default::default()
        };
        let mobjs = [thing_at(160, 0), thing_at(200, 0)];

        things.r_add_sprites(&mut sector, &mobjs, &rmain, &rdata, &view);
        things.r_add_sprites(&mut sector, &mobjs, &rmain, &rdata, &view);

        assert_eq!(things.vissprites.len(), 2);
        assert_eq!(things.spritelights, 15);
    }
}
