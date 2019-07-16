//! Raw hardware transactional memory primitives.

#[inline]
fn unsupported() -> ! {
    panic!("target CPU does not support hardware transactional memory")
}

#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub(super) struct BeginCode(i8);

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
pub(super) struct TestCode(i8);

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

#[inline]
pub(super) unsafe fn begin() -> BeginCode {
    unsupported()
}

#[inline]
pub(super) unsafe fn abort() -> ! {
    unsupported()
}

#[inline]
pub(super) unsafe fn test() -> TestCode {
    unsupported()
}

#[inline]
pub(super) unsafe fn end() {
    unsupported()
}

#[inline]
pub(super) const fn htm_supported() -> bool {
    false
}
