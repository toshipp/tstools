#[macro_export]
macro_rules! check_len {
    ($b:expr, $l:expr) => {
        if $b < $l {
            bail!(
                "{}: too short {}({}), expect {}({})",
                line!(),
                stringify!($b),
                $b,
                stringify!($l),
                $l
            );
        }
    };
}
