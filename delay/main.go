package delay

import (
	"bufio"
	"bytes"
	"flag"
	"fmt"
	"io"
	"log"
	"os"

	"../libts"
)

const oneseg_pid = 0x1fc8
const sequence_start_code = 0x1b3

var (
	debug bool
)

func findAVPID(reader io.Reader) (audio_pid uint16, video_pid uint16, err error) {
	var pmt_pid uint16
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
		if pmt_pid == packet.PID {
			pmtd.Submit(packet)
		}
		if audio_pid != 0 && video_pid != 0 {
			return
		}
	}
}

func findAVPTS(audio_pid uint16, video_pid uint16, reader io.Reader) (audio_pts uint64, video_pts uint64, err error) {
	found_audio_pts := false
	found_video_pts := false

	var aph *libts.PESPacketHeader
	apd := libts.NewPESPacketDecoder(
		func(header *libts.PESPacketHeader) {
			if debug {
				log.Print("found audio pes")
			}
			aph = header
			if pts, ok := header.GetPTS(); ok {
				if debug {
					log.Printf("audio pts: %v", pts)
				}
				if !found_audio_pts {
					audio_pts = pts
					found_audio_pts = true
				}
			}
		},
		func(data []byte) {
			if debug {
				log.Printf("found audio pes body, len: %v, packlen: %v",
					len(data), aph.PESPacketLength)
			}
		})

	var vph *libts.PESPacketHeader
	var start_code uint32
	var rest_len int
	vpd := libts.NewPESPacketDecoder(
		func(header *libts.PESPacketHeader) {
			if debug {
				log.Print("found video pes header")
			}
			vph = header
			start_code = 0
			rest_len = 4
		},
		func(data []byte) {
			if debug {
				log.Printf("found video pes body, len: %v", len(data))
			}
			if found_video_pts {
				return
			}
			if !vph.DataAlignmentIndicator && rest_len == 4 {
				return
			}
			for ; rest_len > 0 && len(data) > rest_len; rest_len-- {
				start_code = start_code<<8 | uint32(data[0])
				data = data[1:]

			}
			if rest_len > 0 {
				return
			}
			if start_code == sequence_start_code {
				if pts, ok := vph.GetPTS(); ok {
					if debug {
						log.Printf("video pts: %v", pts)
					}
					video_pts = pts
					found_video_pts = true
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
		if audio_pid == packet.PID {
			apd.Submit(packet)
		}
		if video_pid == packet.PID {
			vpd.Submit(packet)
		}
		if found_video_pts && found_audio_pts {
			return
		}
	}
}

func Main(args []string) {
	commandLine := flag.NewFlagSet("delay", flag.ExitOnError)
	commandLine.BoolVar(&debug, "debug", false, "enable debugging")
	commandLine.Parse(args)
	args = commandLine.Args()
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

	buffered := new(bytes.Buffer)
	ahead_in := io.TeeReader(in, buffered)
	audio_pid, video_pid, e := findAVPID(ahead_in)
	if debug {
		log.Printf("audio: %v, video: %v", audio_pid, video_pid)
	}
	if e != nil {
		log.Fatal(e)
	}

	full_in := io.MultiReader(buffered, in)
	audio_pts, video_pts, e := findAVPTS(audio_pid, video_pid, full_in)
	if e != nil {
		log.Fatal(e)
	}
	diff := video_pts - audio_pts
	if diff > 10*90*1000 {
		log.Fatalf("Too large diff: %v", diff)
	}
	fmt.Printf("%f\n", float64(diff)/90/1000)
}