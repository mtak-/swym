pub mod debug_alloc;
pub mod dyn_vec;
#[macro_use]
pub mod fvec;

pub use self::{dyn_vec::DynVec, fvec::FVec};

#[cfg(feature = "debug-alloc")]
pub type DefaultAlloc = self::debug_alloc::DebugAlloc<std::alloc::Global>;
#[cfg(not(feature = "debug-alloc"))]
pub type DefaultAlloc = std::alloc::Global;
