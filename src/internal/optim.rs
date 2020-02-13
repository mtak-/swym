macro_rules! assume {
    ($e:expr $(, $t:tt)*) => {
        if cfg!(debug_assertions) {
            assert!($e $(, $t)*)
        } else {
            nudge::assume($e)
        }
    };
}
