include!(env!("BINDINGS"));

struct Component;

export!(Component);

impl crate::exports::my::test::i::Guest for Component {
    async fn return_string() -> String {
        "hello".to_string()
    }
}
