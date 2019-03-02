use arib_symbols;

#[test]
fn test() {
    assert_eq!(
        arib_symbols::code_point_to_char(0x7a21).unwrap(),
        '\u{26cc}'
    );
    assert_eq!(
        arib_symbols::code_point_to_char(0x7b46).unwrap(),
        '\u{26f7}'
    );
    assert_eq!(
        arib_symbols::code_point_to_char(0x7d5c).unwrap(),
        '\u{2150}'
    );
    assert_eq!(
        arib_symbols::code_point_to_char(0x7e7d).unwrap(),
        '\u{325b}'
    );
}
