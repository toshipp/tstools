#!/usr/bin/python3

import sys
import textwrap


def rust_indent(s):
    return textwrap.indent(s, " "*4)


def rust_unicode_literal(c):
    return r"'\u{{{:04x}}}'".format(c)


def ucp_to_chr(ucp):
    return chr(int(ucp[2:-1], base=16))


def sjis_to_kt(sjis):
    parts = sjis.split("\\x")
    b1 = int(parts[1], base=16)
    b2 = int(parts[2], base=16)

    even = b2 > 158
    if even:
        t = b2 - 158
    else:
        if b2 < 128:
            t = b2 - 63
        else:
            t = b2 - 64

    if b1 < 224:
        k = b1*2 - 257
    else:
        k = b1*2 - 385

    if even:
        k += 1

    assert(90 <= k <= 94)
    assert(1 <= t <= 94)

    return (k, t)


def read_ucm(inf):
    al = []
    for l in inf:
        if l.startswith("#"):
            continue

        ucp, sjis, _ = l.split(maxsplit=2)
        c = ucp_to_chr(ucp)
        k, t = sjis_to_kt(sjis)
        al.append(((k, t), c))

    return sorted(al, key=lambda x: x[0])


def gen(al, outf):
    print("const TABLE: &[char] = &[", file=outf)

    assert(al[0][0][0] == 90)

    prev_t = 94
    for (k, t), c in al:
        # fill discontinued region
        for _ in range(((94 + t - prev_t) % 94 - 1)):
            print(rust_indent(rust_unicode_literal(0)) + ",", file=outf)

        print(rust_indent(rust_unicode_literal(ord(c))) + ",", file=outf)

        assert(t != prev_t)
        prev_t = t

    print("];\n", file=outf)

    func_str = """\
pub fn code_point_to_char(cp: u16) -> Option<char> {
    let row = cp >> 8;
    let col = cp & 0xff;
    if row < 0x7a || row > 0x7e {
        return None;
    }
    if col < 0x21 || col > 0x7e {
        return None;
    }
    let pos = usize::from((row - 0x7a) * 94 + (col - 0x21));
    if pos >= TABLE.len() {
        return None;
    }
    let c = TABLE[pos];
    if c as usize == 0 {
        return None;
    }
    return Some(c);
}"""
    print(func_str, file=outf)


def main():
    al = read_ucm(sys.stdin)
    gen(al, sys.stdout)


if __name__ == '__main__':
    main()
