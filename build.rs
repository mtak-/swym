macro_rules! stats {
    ($($(#[$attr:meta])* $names:ident: $kinds:tt @ $env_var:ident),* $(,)*) => {
        concat!($("cargo:rerun-if-env-changed=", "SWYM_", stringify!($env_var),"\n"),*)
    };
}

fn main() {
    println!(include!("./src/stats_list.rs"));
}
