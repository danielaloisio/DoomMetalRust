//! Library root for DoomMetalRust.
//!
//! Exists so the engine's modules are reachable both from `main.rs` (the
//! `i_main.c` port, see its own docs) and from `tests/` integration
//! tests (which can only see a crate's `pub` surface, not a binary
//! crate's private `mod` tree) — without this, integration tests would
//! have no way to reach modules like `w_wad` against real files (e.g.
//! the real `doom.wad`) short of duplicating them.
//!
//! `dead_code` is allowed crate-wide for now: these modules port whole
//! original headers' worth of API surface (constants, enums, functions)
//! ahead of the later phases that will actually call into them. Remove
//! this once enough of the engine is wired together that unused ported
//! symbols become a meaningful signal again.
#![allow(dead_code)]

pub mod am_map;
pub mod d_englsh;
pub mod d_event;
pub mod d_items;
pub mod d_main;
pub mod d_net;
pub mod d_player;
pub mod d_think;
pub mod d_ticcmd;
pub mod doomdata;
pub mod doomdef;
pub mod doomstat;
pub mod doomtype;
pub mod dstrings;
pub mod f_finale;
pub mod f_wipe;
pub mod g_game;
pub mod global_cell;
pub mod hu_lib;
pub mod hu_stuff;
pub mod hu_tables;
pub mod i_net;
pub mod i_sound;
pub mod i_system;
pub mod i_video;
pub mod info;
pub mod m_argv;
pub mod m_bbox;
pub mod m_cheat;
pub mod m_fixed;
pub mod m_menu;
pub mod m_misc;
pub mod m_random;
pub mod m_swap;
pub mod p_ceilng;
pub mod p_doors;
pub mod p_enemy;
pub mod p_floor;
pub mod p_inter;
pub mod p_lights;
pub mod p_map;
pub mod p_maputl;
pub mod p_mobj;
pub mod p_plats;
pub mod p_pspr;
pub mod p_saveg;
pub mod p_setup;
pub mod p_sight;
pub mod p_spec;
pub mod p_switch;
pub mod p_telept;
pub mod p_tick;
pub mod p_user;
pub mod r_bsp;
pub mod r_data;
pub mod r_defs;
pub mod r_draw;
pub mod r_main;
pub mod r_plane;
pub mod r_segs;
pub mod r_sky;
pub mod r_state;
pub mod r_things;
pub mod s_sound;
pub mod sounds;
pub mod st_lib;
pub mod st_stuff;
pub mod tables;
pub mod v_video;
pub mod w_wad;
pub mod wi_stuff;
pub mod z_zone;
