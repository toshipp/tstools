package libts

import (
	"errors"
	"fmt"
	"io"
)

func makeTable() []uint32 {
	t := make([]uint32, 256)
	for i := 0; i < 256; i++ {
		crc := uint32(i) << 24
		for j := 0; j < 8; j++ {
			if crc & 0x80000000 != 0 {
				crc = crc << 1 ^ 0x04c11db7
			} else {
				crc <<= 1
			}
		}
		t[i] = crc
	}
	return t
}
var crc32table = makeTable()

const (
	PAT_PID = 0
)

// bigendian crc32
func Crc32(data []byte) uint32 {
	crc := ^uint32(0)
	for _, x := range data {
		i := byte(crc >> 24) ^ x
		crc = crc32table[i] ^ (crc << 8)
	}
	return crc
}

func ReadPID(buffer []byte) uint16 {
	return (uint16(buffer[0])<<8 | uint16(buffer[1])) & 0x1fff
}

func ReadLength(buffer []byte) uint {
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
	DataBytes                  []byte
	RawData                    []byte
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
	packet.PID = ReadPID(data[1:])
	packet.TransportScramblingControl = data[3] >> 6
	packet.AdaptationFieldControl = data[3] & 0x30 >> 4
	packet.ContinuityCounter = data[3] & 0x0f
	packet.RawData = data
	data_byte := data[4:]
	if packet.HasAdaptationField() {
		length := data_byte[0]
		packet.AdaptationField, _ = ParseAdaptationField(data_byte[:1+length])
		data_byte = data_byte[1+length:]
	}
	if packet.HasDataBytes() {
		packet.DataBytes = data_byte
	}
	return packet, nil
}

func (tsp *TSPacket) HasAdaptationField() bool {
	return tsp.AdaptationFieldControl == 2 ||
		tsp.AdaptationFieldControl == 3
}

func (tsp *TSPacket) HasDataBytes() bool {
	return tsp.AdaptationFieldControl == 1 ||
		tsp.AdaptationFieldControl == 3
}

func (pr *PacketReader) ReadPacket() (*TSPacket, error) {
	buf := make([]byte, 188)
	done := 0
	for done < len(buf) {
		n, e := pr.reader.Read(buf[done:])
		if e != nil {
			return nil, e
		}
		done += n
	}
	return ParsePacket(buf)
}

type ProgramAssotiation struct {
	ProgramNumber uint16
	PID           uint16 /* network PID or program map PID */
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

type SectionParseCallback func([] byte)

type SectionDecoder struct {
	started  bool
	buffer   []byte
	callback SectionParseCallback
}


func NewSectionDecoder(callback SectionParseCallback) *SectionDecoder {
	return &SectionDecoder{false, nil, callback}
}

func (d *SectionDecoder) Submit(packet *TSPacket) {
	if packet.PayloadUnitStart {
		d.started = true
		pointer_field := packet.DataBytes[0]
		d.buffer = packet.DataBytes[pointer_field+1:]
	} else if d.started {
		d.buffer = append(d.buffer, packet.DataBytes...)
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
		section_length := ReadLength(d.buffer[1:])
		whole_sec_len := section_length + 3
		if uint(len(d.buffer)) < whole_sec_len {
			// not enough to parse desc
			break
		}
		d.callback(d.buffer[:whole_sec_len])
		d.buffer = d.buffer[whole_sec_len:]
	}
}

const PATHeaderLen uint = 8
const CRC32Len uint = 4

func NewPATSectionDecoder(callback func(*PATSection)) *SectionDecoder {
	return NewSectionDecoder(func(buffer []byte) {
		assoc_len := (uint(len(buffer)) - PATHeaderLen - CRC32Len) / 4
		assocs := make([]ProgramAssotiation, assoc_len)
		for i := uint(0); i < assoc_len; i++ {
			p := PATHeaderLen + i*4
			assocs[i].ProgramNumber = uint16(buffer[p])<<8 | uint16(buffer[p+1])
			assocs[i].PID = ReadPID(buffer[p+2:])
		}
		section := &PATSection{
			CreateSectionHeaderFromBuffer(buffer),
			assocs,
		}
		callback(section)
	})
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
	ElementaryPID uint16
	ESInfo         []Descriptor
}

func (s *StreamInfo) Print() {
	fmt.Printf(`stream type: %d
elementary PID: %d
descriptors:
`, s.StreamType, s.ElementaryPID)
	for _, desc := range s.ESInfo {
		desc.Print()
	}
}

type PMTSection struct {
	SectionHeader
	PCR_PID     uint16
	ProgramInfo []Descriptor
	StreamInfo  []StreamInfo
}

func (p *PMTSection) Print() {
	p.SectionHeader.Print()
	fmt.Printf("PCR PID: %d\n", p.PCR_PID)
	fmt.Printf("descriptors:\n")
	for _, desc := range p.ProgramInfo {
		desc.Print()
	}
	fmt.Printf("stream info:\n")
	for _, si := range p.StreamInfo {
		si.Print()
	}
}

func NewPMTSectionDecoder(callback func(*PMTSection)) *SectionDecoder {
	return NewSectionDecoder(func(buffer []byte) {
		pcr_pid := ReadPID(buffer[8:])
		program_info_length := ReadLength(buffer[10:])
		// todo decode program info
		program_info := make([]Descriptor, 0)
		stream_info := make([]StreamInfo, 0)
		for p := 12 + program_info_length; p < uint(len(buffer))-CRC32Len; {
			es_info_length := ReadLength(buffer[p+3:])
			es_info := make([]Descriptor, 0)
			stream_info = append(stream_info,
				StreamInfo{
					uint8(buffer[p]),
					ReadPID(buffer[p+1:]),
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
	})
}
