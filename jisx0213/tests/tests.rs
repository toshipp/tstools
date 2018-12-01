extern crate jisx0213;

#[test]
fn test() {
    // 1-82-02
    assert_eq!(
        jisx0213::code_point_to_chars(0x17222).unwrap()[0],
        '\u{9B06}'
    );

    // 1-4-91
    assert_eq!(
        jisx0213::code_point_to_chars(0x1247b).unwrap(),
        ['\u{3053}', '\u{309a}']
    );

    // 2-01-20
    assert_eq!(
        jisx0213::code_point_to_chars(0x22134).unwrap()[0],
        '\u{4EB9}'
    );

    // 2-84-03
    assert_eq!(
        jisx0213::code_point_to_chars(0x27423).unwrap()[0],
        '\u{7CD7}'
    );
}
