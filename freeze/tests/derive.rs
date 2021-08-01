#[derive(freeze::Freeze)]
struct Bar(i32);

fn is_freeze<T: freeze::Freeze>() {}

#[test]
fn foo() {
    is_freeze::<Bar>()
}

mod test {
    use freeze::Freeze;

    #[derive(Freeze)]
    struct Baz(i32);

    #[test]
    fn foo() {
        super::is_freeze::<Baz>()
    }
}
