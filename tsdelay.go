package main

import (
	"./libts"
	"bufio"
	"flag"
	"fmt"
	"io"
	"log"
	"os"
)

const oneseg_pid = 0x1fc8
const sequence_start_code = 0x1b3

func main() {
	flag.Parse()
	args := flag.Args()
	if len(args) > 1 {
		log.Fatal("Invalid number of arguments")
	}
	inf := os.Stdin
	if len(args) >= 1 && args[0] != "-" {
		var e error
		inf, e = os.Open(args[0])
		if e != nil {
			log.Fatal(e)
		}
		defer inf.Close()
	}
	in := bufio.NewReader(inf)

	reader := libts.NewPacketReader(in)
	pmt_pid := uint16(0)
	audio_pid := uint16(0)
	video_pid := uint16(0)
	patd := libts.NewPATSectionDecoder(func(sec *libts.PATSection) {
		for _, assoc := range sec.Assotiations {
			if assoc.ProgramNumber != 0 &&
				assoc.PID != oneseg_pid {
				pmt_pid = assoc.PID
				return
			}
		}
	})
	pmtd := libts.NewPMTSectionDecoder(func(sec *libts.PMTSection) {
		for _, info := range sec.StreamInfo {
			switch info.StreamType {
			case libts.StreamType_AAC_ADTS:
				audio_pid = info.ElementaryPID
			case libts.StreamType_H262:
				video_pid = info.ElementaryPID
			}
		}
	})

	audio_pts := ^uint64(0)
	video_pts := ^uint64(0)
	apd := libts.NewPESPacketDecoder(func(header libts.PESPacketHeader) {
		if audio_pts != ^uint64(0) {
			return
		}
		if pts, ok := header.GetPTS(); ok {
			audio_pts = pts
		}
	}, nil)
	var vph *libts.PESPacketHeader = nil
	start_code := uint32(0)
	read_len := 0
	vpd := libts.NewPESPacketDecoder(
		func(header libts.PESPacketHeader) {
			vph = &header
			start_code = 0
			read_len = 0
		},
		func(data []byte) {
			if video_pts != ^uint64(0) {
				return
			}
			if !vph.DataAlignmentIndicator || read_len == 4 {
				return
			}
			l := len(data)
			if l > 4-read_len {
				l = 4 - read_len
			}
			for i := 0; i < l; i++ {
				start_code = start_code<<8 | uint32(data[i])
			}
			read_len += l
			if read_len == 4 {
				if start_code == sequence_start_code {
					if pts, ok := vph.GetPTS(); ok {
						video_pts = pts
					}
				}
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
		}
		if pmt_pid == packet.PID {
			pmtd.Submit(packet)
		}
		if audio_pid == packet.PID {
			apd.Submit(packet)
		}
		if video_pid == packet.PID {
			vpd.Submit(packet)
		}
		if audio_pts != ^uint64(0) && video_pts != ^uint64(0) {
			diff := video_pts - audio_pts
			fmt.Printf("%f\n", float64(diff)/90/1000)
			return
		}
	}
}
