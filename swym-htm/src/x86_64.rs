//! Raw x86_64 Restricted Transactional Memory primitives.
//!
//! To enable this module use the `--features rtm` switch.
//!
//! A compatible CPU is required. Additionally these `RUSTFLAGS` may be necessary.
//!
//! ```sh
//! RUSTFLAGS="-Ctarget-cpu=native -Ctarget-feature=+rtm"
//! ```
//!
//! The [Intel programming considerations](https://software.intel.com/en-us/cpp-compiler-developer-guide-and-reference-intel-transactional-synchronization-extensions-intel-tsx-programming-considerations)
//! are a recommended read for using this module.

mod intrinsics {
    extern "C" {
        #[link_name = "llvm.x86.xbegin"]
        pub fn xbegin() -> i32;

        #[link_name = "llvm.x86.xend"]
        pub fn xend() -> ();

        #[link_name = "llvm.x86.xabort"]
        pub fn xabort(a: i8) -> ();

        #[link_name = "llvm.x86.xtest"]
        pub fn xtest() -> i32;
    }
}

/// Abort codes must be immediates.
///
/// There's no way to represent immediates in rust at the moment, but this trait works in practice.
pub trait XAbortConst {
    /// The code with which to abort.
    const CODE: i8;
}

/// Specifies the start of a restricted transactional memory (RTM) code region and returns a value
/// indicating status.
///
/// See the [Intel documentation](https://software.intel.com/en-us/cpp-compiler-developer-guide-and-reference-xbegin).
#[inline]
pub unsafe fn xbegin() -> i32 {
    intrinsics::xbegin()
}

/// Specifies the end of a restricted transactional memory (RTM) code region.
///
/// See the [Intel documentation](https://software.intel.com/en-us/cpp-compiler-developer-guide-and-reference-xend).
#[inline]
pub unsafe fn xend() {
    intrinsics::xend()
}

/// Forces a restricted transactional memory (RTM) region to abort.
///
/// See the [Intel documentation](https://software.intel.com/en-us/cpp-compiler-developer-guide-and-reference-xabort).
#[inline(always)]
pub unsafe fn xabort<T: XAbortConst>() -> ! {
    intrinsics::xabort(T::CODE);
    core::hint::unreachable_unchecked()
}

/// Queries whether the processor is executing in a transactional region identified by restricted
/// transactional memory (RTM) or hardware lock elision (HLE).
///
/// See the [Intel documentation](https://software.intel.com/en-us/cpp-compiler-developer-guide-and-reference-xtest).
#[inline]
pub unsafe fn xtest() -> i32 {
    intrinsics::xtest()
}

/// Transaction successfully started.
pub const _XBEGIN_STARTED: i32 = !0 as i32;

/// Transaction explicitly aborted with xabort. The parameter passed to xabort is available with
/// _XABORT_CODE(status).
pub const _XABORT_EXPLICIT: i32 = 1i32 << 0;

/// Transaction retry is possible.
pub const _XABORT_RETRY: i32 = 1i32 << 1;

/// Transaction abort due to a memory conflict with another thread.
pub const _XABORT_CONFLICT: i32 = 1i32 << 2;

/// Transaction abort due to the transaction using too much memory.
pub const _XABORT_CAPACITY: i32 = 1i32 << 3;

/// Transaction abort due to a debug trap.
pub const _XABORT_DEBUG: i32 = 1i32 << 4;

/// Transaction abort in a inner nested transaction.
pub const _XABORT_NESTED: i32 = 1i32 << 5;

/// Retrieves the parameter passed to xabort when status has the `_XABORT_EXPLICIT` flag set.
#[allow(non_snake_case)]
#[inline(always)]
pub const fn _XABORT_CODE(status: i32) -> i32 {
    ((status as u32 >> 24) & 0xFFu32) as i32
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub(super) struct BeginCode(i32);

impl BeginCode {
    #[inline]
    pub fn is_started(&self) -> bool {
        self.0 == _XBEGIN_STARTED
    }

    #[inline]
    pub fn is_explicit_abort(&self) -> bool {
        self.0 & _XABORT_EXPLICIT != 0
    }

    #[inline]
    pub fn is_retry(&self) -> bool {
        self.0 & _XABORT_RETRY != 0
    }

    #[inline]
    pub fn is_conflict(&self) -> bool {
        self.0 & _XABORT_CONFLICT != 0
    }

    #[inline]
    pub fn is_capacity(&self) -> bool {
        self.0 & _XABORT_CONFLICT != 0
    }
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub(super) struct TestCode(i32);

impl TestCode {
    #[inline]
    pub fn in_transaction(&self) -> bool {
        self.0 != 0
    }

    #[inline]
    pub fn is_suspended(&self) -> bool {
        false
    }
}

#[inline]
pub(super) unsafe fn begin() -> BeginCode {
    BeginCode(xbegin())
}

#[inline(always)]
pub(super) unsafe fn abort() -> ! {
    struct Code;
    impl XAbortConst for Code {
        const CODE: i8 = 0;
    }
    xabort::<Code>();
}

#[inline]
pub(super) unsafe fn test() -> TestCode {
    TestCode(xtest())
}

#[inline]
pub(super) unsafe fn end() {
    xend()
}

#[inline]
pub(super) const fn htm_supported() -> bool {
    true
}

#[inline]
pub(super) fn htm_supported_runtime() -> bool {
    unsafe { core::arch::x86_64::__cpuid_count(0x7, 0x0).ebx & (1 << 11) != 0 }
}
