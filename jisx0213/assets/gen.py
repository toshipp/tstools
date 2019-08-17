#!/usr/bin/python3

import sys
import textwrap


def rust_indent(s):
    return textwrap.indent(s, " "*4)


def rust_unicode_literal(c):
    return r"'\u{{{:04x}}}'".format(c)


def gen_src(inf, outf):
    prev_row = 0
    prev_col = 0x7e
    block_start_rows = []
    block_end_rows = []
    multi_chars = []

    print("const TABLE: &[char] = &[", file=outf)

    for l in inf:
        parts = [int(x, 16) for x in l.strip().split()]
        assert(2 <= len(parts) <= 3)
        code_point = parts[0]
        row = code_point >> 8
        col = code_point & 0xff
        assert(0x21 <= col <= 0x7e)

        if row != prev_row and row != prev_row + 1:
            block_start_rows.append(row)
            if prev_row != 0:
                block_end_rows.append(prev_row)

        # fill discontinued cols
        for _ in range(((94 + col - prev_col) % 94 - 1)):
            print(rust_indent(rust_unicode_literal(0)) + ",", file=outf)

        prev_row = row
        prev_col = col

        if len(parts) == 2:
            print(rust_indent(rust_unicode_literal(parts[1]) + ","), file=outf)
        else:
            multi_chars.append(parts[1:])
            c = len(multi_chars)
            print(rust_indent(rust_unicode_literal(c) + ","), file=outf)

    block_end_rows.append(row)

    assert(len(block_start_rows) == len(block_end_rows))

    print("];\n", file=outf)

    print("const MULTI_CHAR_TABLE: &[[char; 2]] = &[", file=outf)
    for mc in multi_chars:
        print(rust_indent("[" + ", ".join(rust_unicode_literal(c) for c in mc) + "],"), file=outf)
    print("];\n", file=outf)

    func_str = """\
pub fn code_point_to_chars(cp: u32) -> Option<&'static [char]> {
    let row = cp >> 8;
    let row = match row {
"""

    offset = 0
    for bs, be in zip(block_start_rows, block_end_rows):
        if bs == be:
            func_str += """\
        0x{bs:x} => row - {offset},
""".format(bs=bs, offset=bs-offset)
        else:
            func_str += """\
        0x{bs:x}..=0x{be:x} => row - {offset},
""".format(bs=bs, be=be, offset=bs-offset)
        offset += be - bs + 1


    func_str += """\
        _ => return None,
    };
    let col = cp & 0xff;
    if col < 0x21 || col > 0x7e {
        return None;
    }
    let col = col - 0x21;
    let offset = (row * 94 + col) as usize;
    let cp = &TABLE[offset..offset + 1];
    let c = cp[0] as usize;
    if c == 0 {
        return None;
    }
    if c >= 0x80 {
        return Some(cp);
    }
    return Some(&MULTI_CHAR_TABLE[c - 1]);
}"""

    print(func_str, file=outf)


def main():
    gen_src(sys.stdin, sys.stdout)


if __name__ == "__main__":
    main()
