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
//  MapObj data. Map Objects or mobjs are actors, entities,
//  thinker, take-your-pick... anything that moves, acts, or
//  suffers state changes of more or less violent nature.
//
//-----------------------------------------------------------------------------

//! Rust port of `d_think.h`.
//!
//! MapObj data. Map Objects or mobjs are actors, entities, thinkers —
//! take your pick — anything that moves, acts, or suffers state changes
//! of more or less violent nature.
//!
//! The original's `actionf_t` is a C `union` of three function pointer
//! shapes (0/1/2 `void*` args), used because a single `think_t` field on
//! `thinker_t`/`mobj_t` is called with different signatures depending on
//! context (state action functions take one or two args, generic
//! thinkers take none). Rust has no direct union-of-fn-pointers
//! equivalent that stays safe to call; the faithful port for "port fiel"
//! is a raw-pointer-sized union via `#[repr(C)]`, preserving the
//! original's "caller and callee must agree out of band on the real
//! signature" contract (including its unsafety) rather than papering
//! over it with a safe enum this early — an enum dispatch is a
//! reasonable target for the idiomatization pass, once the actual set of
//! action functions is ported and their call sites are known.

/// (`actionf_v`) — no-argument thinker function.
pub type ActionFV = unsafe extern "C" fn();
/// (`actionf_p1`) — one-argument action function (e.g. `mobj_t*`).
pub type ActionFP1 = unsafe extern "C" fn(*mut std::ffi::c_void);
/// (`actionf_p2`) — two-argument action function (e.g. `player_t*,
/// pspdef_t*`).
pub type ActionFP2 = unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void);

/// (`actionf_t`) — union of the three action function pointer shapes.
///
/// Mirrors the original C `union`: exactly one field is meaningful at a
/// time, chosen by the calling convention the surrounding code expects
/// for that particular thinker/state. All three pointer types are the
/// same size, so this is a safe `#[repr(C)]` union to declare (reading
/// the "wrong" variant is exactly as unsafe here as it was implicitly in
/// the original C, no more, no less).
#[repr(C)]
#[derive(Clone, Copy)]
pub union ActionF {
    pub acv: ActionFV,
    pub acp1: ActionFP1,
    pub acp2: ActionFP2,
}

/// (`think_t`) — historically, "think_t" is yet another function pointer
/// to a routine to handle an actor.
pub type Think = ActionF;

/// (`thinker_t`) — doubly linked list of actors.
///
/// The original stores raw `thinker_s*` prev/next pointers threaded
/// through a circular list with a sentinel head (see `p_tick.c`, ported
/// in a later phase). Kept as raw pointers here rather than introducing
/// `Rc`/indices, since the linked-list traversal/splice logic that gives
/// those pointers meaning hasn't been ported yet — changing the
/// representation ahead of that logic would just be guessing.
#[repr(C)]
pub struct Thinker {
    pub prev: *mut Thinker,
    pub next: *mut Thinker,
    pub function: Think,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    unsafe extern "C" fn noop() {}

    #[test]
    fn actionf_union_variants_are_same_size() {
        assert_eq!(size_of::<ActionFV>(), size_of::<ActionFP1>());
        assert_eq!(size_of::<ActionFP1>(), size_of::<ActionFP2>());
    }

    #[test]
    fn actionf_can_store_and_call_a_v_function() {
        let action = ActionF { acv: noop };
        unsafe {
            (action.acv)();
        }
    }

    #[test]
    fn thinker_prev_next_default_to_null() {
        // No Default derive (raw pointer union field can't derive it),
        // so this just documents/checks the expected null-initialized
        // shape ported code will construct by hand.
        let t = Thinker {
            prev: std::ptr::null_mut(),
            next: std::ptr::null_mut(),
            function: ActionF { acv: noop },
        };
        assert!(t.prev.is_null());
        assert!(t.next.is_null());
    }
}
