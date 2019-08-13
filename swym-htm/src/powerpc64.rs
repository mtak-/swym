//! Raw powerpc64 hardware transactional memory primitives.

mod intrinsics {
    extern "C" {
        #[link_name = "llvm.ppc.tbegin"]
        pub fn tbegin(b: i32) -> i32;

        #[link_name = "llvm.ppc.tend"]
        pub fn tend(e: i32) -> i32;

        #[link_name = "llvm.ppc.ttest"]
        pub fn ttest() -> i64;

        #[link_name = "llvm.ppc.tabort"]
        pub fn tabort(a: i32) -> i32;

        #[link_name = "llvm.ppc.tabortdc"]
        pub fn tabortdc(a: i32, b: i32, c: i32) -> i32;

        #[link_name = "llvm.ppc.tabortdci"]
        pub fn tabortdci(a: i32, b: i32, c: i32) -> i32;

        #[link_name = "llvm.ppc.tabortwc"]
        pub fn tabortwc(a: i32, b: i32, c: i32) -> i32;

        #[link_name = "llvm.ppc.tabortwci"]
        pub fn tabortwci(a: i32, b: i32, c: i32) -> i32;

        #[link_name = "llvm.ppc.tendall"]
        pub fn tendall() -> i32;

        #[link_name = "llvm.ppc.tresume"]
        pub fn tresume() -> i32;

        #[link_name = "llvm.ppc.tsuspend"]
        pub fn tsuspend() -> i32;

        #[link_name = "llvm.ppc.set.texasr"]
        pub fn set_texasr(exasr: i64) -> ();

        #[link_name = "llvm.ppc.set.texasru"]
        pub fn set_texasru(exasru: i64) -> ();

        #[link_name = "llvm.ppc.set.tfhar"]
        pub fn set_tfhar(fhar: i64) -> ();

        #[link_name = "llvm.ppc.set.tfiar"]
        pub fn set_tfiar(fiar: i64) -> ();

        #[link_name = "llvm.ppc.get.texasr"]
        pub fn get_texasr() -> i64;

        #[link_name = "llvm.ppc.get.texasru"]
        pub fn get_texasru() -> i64;

        #[link_name = "llvm.ppc.get.tfhar"]
        pub fn get_tfhar() -> i64;

        #[link_name = "llvm.ppc.get.tfiar"]
        pub fn get_tfiar() -> i64;
    }
}

#[inline(always)]
pub unsafe fn tbegin(b: i32) -> i32 {
    intrinsics::tbegin(b)
}

#[inline(always)]
pub unsafe fn tend(e: i32) -> i32 {
    intrinsics::tend(e)
}

#[inline(always)]
pub unsafe fn ttest() -> i64 {
    intrinsics::ttest()
}

#[inline(always)]
pub unsafe fn tabort(a: i32) -> i32 {
    intrinsics::tabort(a)
}

#[inline(always)]
pub unsafe fn tabortdc(a: i32, b: i32, c: i32) -> i32 {
    intrinsics::tabortdc(a, b, c)
}

#[inline(always)]
pub unsafe fn tabortdci(a: i32, b: i32, c: i32) -> i32 {
    intrinsics::tabortdci(a, b, c)
}

#[inline(always)]
pub unsafe fn tabortwc(a: i32, b: i32, c: i32) -> i32 {
    intrinsics::tabortwc(a, b, c)
}

#[inline(always)]
pub unsafe fn tabortwci(a: i32, b: i32, c: i32) -> i32 {
    intrinsics::tabortwci(a, b, c)
}

#[inline]
pub unsafe fn tendall() -> i32 {
    intrinsics::tendall()
}

#[inline]
pub unsafe fn tresume() -> i32 {
    intrinsics::tresume()
}

#[inline]
pub unsafe fn tsuspend() -> i32 {
    intrinsics::tsuspend()
}

#[inline(always)]
pub unsafe fn set_texasr(exasr: i64) -> () {
    intrinsics::set_texasr(exasr)
}

#[inline(always)]
pub unsafe fn set_texasru(exasru: i64) -> () {
    intrinsics::set_texasr(exasru)
}

#[inline(always)]
pub unsafe fn set_tfhar(fhar: i64) -> () {
    intrinsics::set_texasr(fhar)
}

#[inline(always)]
pub unsafe fn set_tfiar(fiar: i64) -> () {
    intrinsics::set_texasr(fiar)
}

#[inline]
pub unsafe fn get_texasr() -> i64 {
    intrinsics::get_texasr()
}

#[inline]
pub unsafe fn get_texasru() -> i64 {
    intrinsics::get_texasru()
}

#[inline]
pub unsafe fn get_tfhar() -> i64 {
    intrinsics::get_tfhar()
}

#[inline]
pub unsafe fn get_tfiar() -> i64 {
    intrinsics::get_tfiar()
}

#[allow(non_snake_case)]
#[inline(always)]
pub const fn _HTM_STATE(CR0: i64) -> i64 {
    ((CR0 >> 1) & 0x3)
}

pub const _HTM_NONTRANSACTIONAL: i64 = 0x0;
pub const _HTM_SUSPENDED: i64 = 0x1;
pub const _HTM_TRANSACTIONAL: i64 = 0x2;

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub(super) struct BeginCode(i32);

impl BeginCode {
    #[inline]
    pub fn is_started(&self) -> bool {
        self.0 != 0
    }

    #[inline]
    pub fn is_explicit_abort(&self) -> bool {
        unimplemented!()
    }

    #[inline]
    pub fn is_retry(&self) -> bool {
        unimplemented!()
    }

    #[inline]
    pub fn is_conflict(&self) -> bool {
        unimplemented!()
    }

    #[inline]
    pub fn is_capacity(&self) -> bool {
        unimplemented!()
    }
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub(super) struct TestCode(i64);

impl TestCode {
    #[inline]
    pub fn in_transaction(&self) -> bool {
        _HTM_STATE(self.0) == _HTM_TRANSACTIONAL
    }

    #[inline]
    pub fn is_suspended(&self) -> bool {
        _HTM_STATE(self.0) == _HTM_SUSPENDED
    }
}

#[inline]
pub(super) unsafe fn begin() -> BeginCode {
    BeginCode(tbegin(0))
}

#[inline(always)]
pub(super) unsafe fn abort() -> ! {
    tabort(0);
    core::hint::unreachable_unchecked()
}

#[inline]
pub(super) unsafe fn test() -> TestCode {
    TestCode(ttest())
}

#[inline]
pub(super) unsafe fn end() {
    tend(0);
}

#[inline]
pub(super) const fn htm_supported() -> bool {
    // TODO:
    false
}
