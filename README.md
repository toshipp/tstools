TSTools
-------
tools collection for mpeg2-ts.

Tools
-----
* tssplit
```
usage: tssplit [infile] [outfile]
```
  Split ts stream from `infile`.
  This dumps modified PAT (removed 1seg PMT), PMT, NIT, and packes associating with leaved PMT.

* tsdelay
```
usage: tsdelay [infile]
```
  Show difference between video and audio start timestamp in second.
