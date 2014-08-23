package main

import (
	"./libts"
	"bufio"
	"bytes"
	"flag"
	"io"
	"log"
	"os"
)

const oneseg_pid = 0x1fc8

type PIDSet map[uint16]struct{}

func NewPIDSet() PIDSet {
	return make(PIDSet)
}
func (s PIDSet) add(pid uint16) {
	s[pid] = struct{}{}
}
func (s PIDSet) find(pid uint16) bool {
	_, ok := s[pid]
	return ok
}
func (s PIDSet) size() int {
	return len(s)
}

func writeall(w io.Writer, data []byte) error {
	done := 0
	for done < len(data) {
		n, e := w.Write(data[done:])
		if e != nil {
			return e
		}
		done += n
	}
	return nil
}

func findKeepPID(reader io.Reader) (keep_pids PIDSet, err error) {
	pmt_pids := NewPIDSet()
	done_pmts := NewPIDSet()
	keep_pids = NewPIDSet()
	pmtds := make(map[uint16]*libts.SectionDecoder)
	new_pmtsd := func(pmt_pid uint16) *libts.SectionDecoder {
		return libts.NewPMTSectionDecoder(func(sec *libts.PMTSection) {
			if done_pmts.find(pmt_pid) {
				return
			}
			keep_pids.add(pmt_pid)
			keep_pids.add(sec.PCR_PID)
			for _, info := range sec.StreamInfo {
				keep_pids.add(info.ElementaryPID)
			}
			done_pmts.add(pmt_pid)
		})
	}
	patd := libts.NewPATSectionDecoder(func(sec *libts.PATSection) {
		for _, assoc := range sec.Assotiations {
			if assoc.ProgramNumber == 0 {
				keep_pids.add(assoc.PID)
			} else if assoc.PID != oneseg_pid {
				if !pmt_pids.find(assoc.PID) {
					pmt_pids.add(assoc.PID)
					pmtds[assoc.PID] = new_pmtsd(assoc.PID)
				}
			}
		}
	})

	pr := libts.NewPacketReader(reader)
	for {
		packet, e := pr.ReadPacket()
		if e != nil {
			err = e
			return
		}
		if packet.PID == libts.PAT_PID {
			patd.Submit(packet)
		}
		if decoder, ok := pmtds[packet.PID]; ok {
			decoder.Submit(packet)
		}
		if pmt_pids.size() > 0 && pmt_pids.size() == done_pmts.size() {
			return
		}
	}
}

func dump_pat(out io.Writer, pat *libts.TSPacket, keep_pids PIDSet) error {
	data := pat.DataBytes
	head_pos := uint(data[0]) + 1
	length_pos := head_pos + 1
	length := libts.ReadLength(data[length_pos:])
	assoc_pos := head_pos + 8
	last_assoc_pos := length_pos + 2 + length - 4
	in_pos := assoc_pos
	out_pos := assoc_pos
	for in_pos < last_assoc_pos {
		prog_num := uint16(data[in_pos])<<8 + uint16(data[in_pos+1])
		pid := libts.ReadPID(data[in_pos+2:])
		if prog_num == 0 || keep_pids.find(pid) {
			if in_pos != out_pos {
				copy(data[out_pos:out_pos+4], data[in_pos:in_pos+4])
			}
			out_pos += 4
		}
		in_pos += 4
	}
	crc := libts.Crc32(data[head_pos:out_pos])
	data[out_pos] = byte(crc >> 24)
	data[out_pos+1] = byte(crc >> 16)
	data[out_pos+2] = byte(crc >> 8)
	data[out_pos+3] = byte(crc)
	data[length_pos+1] = byte(out_pos + 4 - length_pos - 2)
	for i := out_pos + 4; i < uint(len(data)); i++ {
		data[i] = 0xff
	}
	return writeall(out, pat.RawData)
}

func dump_ts(keep_pids PIDSet, in io.Reader, out io.Writer) error {
	pr := libts.NewPacketReader(in)
	for {
		packet, e := pr.ReadPacket()
		if e != nil {
			if e == io.EOF {
				return nil
			}
			return e
		}
		if packet.PID == libts.PAT_PID {
			e = dump_pat(out, packet, keep_pids)
		} else if keep_pids.find(packet.PID) {
			e = writeall(out, packet.RawData)
		}
		if e != nil {
			return e
		}
	}
}

func main() {
	flag.Parse()
	args := flag.Args()
	if len(args) > 2 {
		log.Fatal("Invalid number of arguments")
	}
	inf := os.Stdin
	outf := os.Stdout
	if len(args) >= 1 && args[0] != "-" {
		var e error
		inf, e = os.Open(args[0])
		if e != nil {
			log.Fatal(e)
		}
		defer inf.Close()
	}
	if len(args) >= 2 && args[1] != "-" {
		var e error
		outf, e = os.Create(args[1])
		if e != nil {
			log.Fatal(e)
		}
		defer outf.Close()
	}
	in := bufio.NewReader(inf)
	out := bufio.NewWriter(outf)
	defer out.Flush()

	buffered := new(bytes.Buffer)
	ahead_in := io.TeeReader(in, buffered)
	keep_pids, e := findKeepPID(ahead_in)
	if e != nil {
		log.Fatal(e)
	}

	full_in := io.MultiReader(buffered, in)
	e = dump_ts(keep_pids, full_in, out)
	if e != nil {
		log.Fatal(e)
	}
}
