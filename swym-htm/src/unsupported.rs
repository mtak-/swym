#[inline]
fn unsupported() -> ! {
    panic!("target CPU does not support hardware transactional memory")
}

#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct BeginCode(i8);

impl BeginCode {
    pub const STARTED: Self = BeginCode(0);
    pub const RETRY: Self = BeginCode(0);
    pub const CONFLICT: Self = BeginCode(0);
    pub const CAPACITY: Self = BeginCode(0);
    pub const DEBUG: Self = BeginCode(0);
    pub const NESTED: Self = BeginCode(0);

    #[inline]
    pub fn is_explicit(&self) -> bool {
        unsupported()
    }

    #[inline]
    pub fn abort_code(&self) -> Option<AbortCode> {
        unsupported()
    }
}

#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct TestCode(i8);

#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct AbortCode(i8);

#[inline]
pub unsafe fn begin() -> BeginCode {
    unsupported()
}

#[inline]
pub unsafe fn abort(_: AbortCode) -> ! {
    unsupported()
}

#[inline]
pub unsafe fn test() -> TestCode {
    unsupported()
}

#[inline]
pub unsafe fn end() {
    unsupported()
}

#[inline]
pub const fn htm_supported() -> bool {
    false
}
