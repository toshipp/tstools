package main

import (
	"./libts"
	"bufio"
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

func dump_pat(out io.Writer, pat *libts.TSPacket, pmts PIDSet) {
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
		if prog_num == 0 || pmts.find(pid) {
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
	writeall(out, pat.RawData)
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

	reader := libts.NewPacketReader(in)
	pmt_pids := NewPIDSet()
	keep_pids := NewPIDSet()
	patd := libts.NewPATSectionDecoder(func(sec *libts.PATSection) {
		for _, assoc := range sec.Assotiations {
			if assoc.ProgramNumber == 0 {
				keep_pids.add(assoc.PID)
			} else if assoc.PID != oneseg_pid {
				keep_pids.add(assoc.PID)
				pmt_pids.add(assoc.PID)
			}
		}
	})
	pmtd := libts.NewPMTSectionDecoder(func(sec *libts.PMTSection) {
		keep_pids.add(sec.PCR_PID)
		for _, info := range sec.StreamInfo {
			keep_pids.add(info.ElementaryPID)
		}
	})

	for {
		packet, e := reader.ReadPacket()
		if e != nil {
			if e == io.EOF {
				break
			}
			log.Fatal(e)
		}
		if packet.PID == libts.PAT_PID {
			patd.Submit(packet)
			dump_pat(out, packet, pmt_pids)
		}
		if pmt_pids.find(packet.PID) {
			pmtd.Submit(packet)
		}
		if keep_pids.find(packet.PID) {
			writeall(out, packet.RawData)
		}
	}
}
