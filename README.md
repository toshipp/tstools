TSTools
-------
tools collection for mpeg2-ts.

Usage
-----

```
tstools split [-debug] [infile] [outfile]
```
Split ts stream from `infile`.
This dumps modified PAT (removed 1seg PMT), PMT, NIT, and packes associating with leaved PMT.

```
tstools delay [-debug] [infile]
```
Show difference between video and audio start timestamp in second.
