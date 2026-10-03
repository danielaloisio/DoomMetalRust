//! Shared helper for the globals strategy described in the porting plan
//! (Phase 2): a single confined `unsafe` chokepoint for the classic
//! engine's big mutable global state groups (`doomstat`, `r_state`,
//! etc.), instead of scattering `unsafe { static mut ... }` access
//! across every call site.
//!
//! Doom's simulation is single-threaded (see `doomstat`'s module docs
//! for the fuller rationale), so a plain `UnsafeCell` behind a
//! `OnceLock` is enough — the only thing standing in the way is that
//! `UnsafeCell<T>` isn't `Sync`, which is required for a `static`.
//! [`GlobalCell`] is a thin wrapper that asserts `Sync` for exactly this
//! single-threaded-by-construction use case, so each state module (e.g.
//! `doomstat::state`/`state_mut`) doesn't need to repeat that unsafe
//! impl and justification itself.

use std::cell::UnsafeCell;

/// A `Sync`-asserting wrapper around `UnsafeCell<T>`, for single global
/// state instances in this (single-threaded) engine port. See the
/// module docs for why this is sound here specifically.
pub struct GlobalCell<T>(UnsafeCell<T>);

// SAFETY: the classic DOOM engine's simulation is single-threaded (see
// module docs) — nothing in this port spawns OS threads that touch game
// state concurrently, so there is no actual data race to prevent. This
// is the one, intentional, documented unsafe assertion the whole
// globals strategy rests on.
unsafe impl<T> Sync for GlobalCell<T> {}

impl<T> GlobalCell<T> {
    pub const fn new(value: T) -> Self {
        GlobalCell(UnsafeCell::new(value))
    }

    /// # Safety
    /// Caller must not alias this with an existing `&mut` reference from
    /// [`GlobalCell::get_mut`] (or another [`GlobalCell::get`]/`get_mut`
    /// pair held concurrently) — same aliasing rules as `UnsafeCell`
    /// itself. In practice, callers should use the owning module's
    /// `state()`/`state_mut()` wrapper functions rather than this
    /// directly.
    pub unsafe fn get(&self) -> &T {
        &*self.0.get()
    }

    /// # Safety
    /// See [`GlobalCell::get`].
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_mut(&self) -> &mut T {
        &mut *self.0.get()
    }
}
