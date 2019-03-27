#[inline]
fn unsupported() -> ! {
    panic!("target CPU does not support hardware transactional memory")
}

#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct BeginCode(i8);

impl BeginCode {
    #[inline]
    pub fn is_started(&self) -> bool {
        unsupported()
    }

    #[inline]
    pub fn is_explicit_abort(&self) -> bool {
        unsupported()
    }

    #[inline]
    pub fn is_retry(&self) -> bool {
        unsupported()
    }

    #[inline]
    pub fn is_conflict(&self) -> bool {
        unsupported()
    }

    #[inline]
    pub fn is_capacity(&self) -> bool {
        unsupported()
    }
}

#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct TestCode(i8);

impl TestCode {
    #[inline]
    pub fn in_transaction(&self) -> bool {
        false
    }

    #[inline]
    pub fn is_suspended(&self) -> bool {
        false
    }
}

#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct AbortCode(i8);

impl AbortCode {
    #[inline]
    pub fn new(code: i8) -> Self {
        AbortCode(code)
    }
}

#[inline]
pub unsafe fn begin() -> BeginCode {
    unsupported()
}

#[inline]
pub unsafe fn abort() -> ! {
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
