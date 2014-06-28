package main

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"log"
	"os"
)

func readPID(buffer []byte) uint16 {
	return (uint16(buffer[0])<<8 | uint16(buffer[1])) & 0x1fff
}

func readLength(buffer []byte) uint {
	return (uint(buffer[0])<<8 | uint(buffer[1])) & 0xfff
}

type PacketReader struct {
	reader io.Reader
}

func NewPacketReader(reader io.Reader) *PacketReader {
	return &PacketReader{reader}
}

// currentry this is a dummy.
type AdaptationField []byte

type TSPacket struct {
	SyncByte                   uint8
	TransportError             bool
	PayloadUnitStart           bool
	TransportPriority          uint8  //1bit
	PID                        uint16 //13bit
	TransportScramblingControl uint8  //2bit
	AdaptationFieldControl     uint8  //2bit
	ContinuityCounter          uint8  //4bit
	AdaptationField            AdaptationField
	DataByte                   []byte
}

func ParseAdaptationField(data []byte) (AdaptationField, error) {
	return data, nil
}

func ParsePacket(data []byte) (*TSPacket, error) {
	packet := new(TSPacket)
	packet.SyncByte = data[0]
	if packet.SyncByte != 0x47 {
		return nil, errors.New("Sync byte is not 0x47")
	}
	packet.TransportError = data[1]&0x80 != 0
	packet.PayloadUnitStart = data[1]&0x40 != 0
	packet.TransportPriority = data[1] & 0x20 >> 5
	packet.PID = (uint16(data[1]) & 0x1f << 8) | uint16(data[2])
	packet.TransportScramblingControl = data[3] >> 6
	packet.AdaptationFieldControl = data[3] & 0x30 >> 4
	packet.ContinuityCounter = data[3] & 0x0f
	data_byte := data[4:]
	if packet.HasAdaptationField() {
		length := data_byte[0]
		packet.AdaptationField, _ = ParseAdaptationField(data_byte[:1+length])
		data_byte = data_byte[1+length:]
	}
	if packet.HasDataByte() {
		packet.DataByte = data_byte
	}
	return packet, nil
}

func (tsp *TSPacket) HasAdaptationField() bool {
	return tsp.AdaptationFieldControl == 2 ||
		tsp.AdaptationFieldControl == 3
}

func (tsp *TSPacket) HasDataByte() bool {
	return tsp.AdaptationFieldControl == 1 ||
		tsp.AdaptationFieldControl == 3
}

func (pr *PacketReader) ReadPacket() (*TSPacket, error) {
	buf := make([]byte, 188)
	n, e := pr.reader.Read(buf)
	if e != nil {
		return nil, e
	}
	if n != 188 {
		return nil, errors.New("Can not read one packet")
	}
	return ParsePacket(buf)
}

type ProgramAssotiation struct {
	ProgramNumber uint16
	PID           uint16 /* network PID / program map PID */
}

func (a *ProgramAssotiation) Print() {
	fmt.Printf("[pn] %d => [pid] %d\n", a.ProgramNumber, a.PID)
}

type SectionHeader struct {
	TableID           uint16
	TransportStreamID uint16
	VersionNumber     uint8
	CurrentNext       bool
	SectionNumber     uint8
	LastSectionNumber uint8
}

func (s *SectionHeader) Print() {
	fmt.Printf(
		`table id: %d
transport stream id: %d
version number: %d
current next: %t
section number: %d
last section number: %d
`, s.TableID, s.TransportStreamID, s.VersionNumber, s.CurrentNext,
		s.SectionNumber, s.LastSectionNumber)
}

func CreateSectionHeaderFromBuffer(buffer []byte) SectionHeader {
	return SectionHeader{
		uint16(buffer[0]),
		uint16(buffer[3])<<8 | uint16(buffer[4]),
		uint8(buffer[5]) & 0x1f >> 1,
		buffer[5]&0x1 != 0,
		buffer[6],
		buffer[7],
	}
}

type PATSection struct {
	SectionHeader
	Assotiations []ProgramAssotiation
}

func (s *PATSection) Print() {
	s.SectionHeader.Print()
	fmt.Printf("assotiations:\n")
	for _, assoc := range s.Assotiations {
		assoc.Print()
	}
}

type SectionDecoder struct {
	started  bool
	buffer   []byte
	callback func([]byte)
}

func (d *SectionDecoder) Pour(packet *TSPacket) {
	if packet.PayloadUnitStart {
		d.started = true
		pointer_field := packet.DataByte[0]
		d.buffer = packet.DataByte[pointer_field+1:]
	} else if d.started {
		d.buffer = append(d.buffer, packet.DataByte...)
	} else {
		return
	}
	for len(d.buffer) > 0 {
		table_id := d.buffer[0]
		if table_id == 0xff {
			// reach stuffing bytes
			d.started = false
			d.buffer = nil
			break
		}
		if len(d.buffer) < 3 {
			// not enough to fetch desc len
			break
		}
		section_length := readLength(d.buffer[1:])
		if uint(len(d.buffer)) < section_length+3 {
			// not enough to parse desc
			break
		}
		d.callback(d.buffer[:section_length])
		d.buffer = d.buffer[section_length:]
	}
}

type PATSectionDecoder struct {
	SectionDecoder
}

const PATHeaderLen uint = 8
const CRC32Len uint = 4

func NewPATSectionDecoder(callback func(*PATSection)) *PATSectionDecoder {
	d := new(PATSectionDecoder)
	d.callback = func(buffer []byte) {
		assoc_len := (uint(len(buffer)) - PATHeaderLen - CRC32Len) / 4
		assocs := make([]ProgramAssotiation, assoc_len)
		for i := uint(0); i < assoc_len; i++ {
			p := PATHeaderLen + i*4
			assocs[i].ProgramNumber = uint16(buffer[p])<<8 | uint16(buffer[p+1])
			assocs[i].PID = readPID(buffer[p+2:])
		}
		section := &PATSection{
			CreateSectionHeaderFromBuffer(buffer),
			assocs,
		}
		callback(section)
	}
	return d
}

type PMTSectionDecoder struct {
	SectionDecoder
}

type Descriptor interface {
	Print()
}

type DummyDescriptor struct{}

func (d *DummyDescriptor) Print() {
	fmt.Printf("dummy descriptor\n")
}

type StreamInfo struct {
	StreamType     uint8
	elementary_PID uint16
	ESInfo         []Descriptor
}

func (s *StreamInfo) Print() {
	fmt.Printf(`stream type: %d
elementary PID: %d
descriptors:
`, s.StreamType, s.elementary_PID)
	for _, desc := range s.ESInfo {
		desc.Print()
	}
}

type PMTSection struct {
	SectionHeader
	PCR_PID     uint16
	programinfo []Descriptor
	streaminfo  []StreamInfo
}

func (p *PMTSection) Print() {
	p.SectionHeader.Print()
	fmt.Printf("PCR PID: %d\n", p.PCR_PID)
	fmt.Printf("descriptors:\n")
	for _, desc := range p.programinfo {
		desc.Print()
	}
	fmt.Printf("stream info:\n")
	for _, si := range p.streaminfo {
		si.Print()
	}
}

func NewPMTSectionDecoder(callback func(*PMTSection)) *PMTSectionDecoder {
	p := new(PMTSectionDecoder)
	p.callback = func(buffer []byte) {
		pcr_pid := readPID(buffer[8:])
		program_info_length := readLength(buffer[10:])
		// todo decode program info
		program_info := make([]Descriptor, 0)
		stream_info := make([]StreamInfo, 0)
		for p := 11 + program_info_length; p < uint(len(buffer))-CRC32Len; {
			es_info_length := readLength(buffer[p+3:])
			es_info := make([]Descriptor, 0)
			stream_info = append(stream_info,
				StreamInfo{
					uint8(buffer[p]),
					readPID(buffer[p+1:]),
					es_info,
				})
			p += 5 + es_info_length
		}
		sec := &PMTSection{
			CreateSectionHeaderFromBuffer(buffer),
			pcr_pid,
			program_info,
			stream_info,
		}
		callback(sec)

	}
	return p
}

type PMTDecoder struct {
	decoders map[uint16]*PMTSectionDecoder
}

func NewPMTDecoder() *PMTDecoder {
	return &PMTDecoder{make(map[uint16]*PMTSectionDecoder)}
}

func (p *PMTDecoder) GetUpdateCallback() func(*PATSection) {
	return func(sec *PATSection) {
		for _, assoc := range sec.Assotiations {
			if _, ok := p.decoders[assoc.PID]; !ok  {
				p.decoders[assoc.PID] = NewPMTSectionDecoder(func(sec *PMTSection) { sec.Print() })
			}
		}
	}
}

func (p *PMTDecoder) Pour(packet *TSPacket) {
	if _, ok := p.decoders[packet.PID]; ok {
		p.decoders[packet.PID].Pour(packet)
	}
}

func main() {
	name := flag.String("filename", "", "input filename")
	flag.Parse()
	file, e := os.Open(*name)
	if e != nil {
		log.Fatal(e)
	}
	reader := NewPacketReader(file)
	//patd := NewPATSectionDecoder(func(p *PATSection) { p.Print() })
	pmtd := NewPMTDecoder()
	patd := NewPATSectionDecoder(pmtd.GetUpdateCallback())
	for {
		packet, e := reader.ReadPacket()
		if e != nil {
			if e == io.EOF {
				break
			}
			log.Fatal(e)
		}
		//fmt.Printf("sync byte: %x\n", packet.SyncByte)
		if packet.PID == 0 {
			patd.Pour(packet)
		}
		pmtd.Pour(packet)
	}
}
