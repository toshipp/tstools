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
			if crc&0x80000000 != 0 {
				crc = crc<<1 ^ 0x04c11db7
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

const (
	// from ISO 13818-1 p.48 Table 2-29
	StreamType_H262     = 0x02
	StreamType_AAC_ADTS = 0x0f
)

// bigendian crc32
func Crc32(data []byte) uint32 {
	crc := ^uint32(0)
	for _, x := range data {
		i := byte(crc>>24) ^ x
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
	if _, e := io.ReadFull(pr.reader, buf); e != nil {
		return nil, e
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

type SectionParseCallback func([]byte)

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
	StreamType    uint8
	ElementaryPID uint16
	ESInfo        []Descriptor
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

//Currently, this does not suport full specification.
type PESPacketHeader struct {
	PacketStartCodePrefix  uint32
	StreamID               uint8
	PESPacketLength        uint16
	DataAlignmentIndicator bool
	pts_dts_flags          uint8
	pts                    uint64
}

const PESPacketMustHeaderLength = 9

const (
	pd_not_started   = iota
	pd_started       = iota
	pd_header_parsed = iota
)

type PESPacketDecoder struct {
	state    int
	buffer   []byte
	onHeader func(*PESPacketHeader)
	onData   func([]byte)
}

func NewPESPacketDecoder(onHeader func(*PESPacketHeader), onData func([]byte)) *PESPacketDecoder {
	return &PESPacketDecoder{pd_not_started, nil, onHeader, onData}
}

func (d *PESPacketDecoder) Submit(packet *TSPacket) {
	if packet.PayloadUnitStart {
		d.state = pd_started
		d.buffer = packet.DataBytes
	} else if d.state == pd_started {
		d.buffer = append(d.buffer, packet.DataBytes...)
	} else if d.state == pd_header_parsed {
		if d.onData != nil {
			d.onData(packet.DataBytes)
		}
		return
	} else {
		// pd_not_started
		return
	}
	if len(d.buffer) >= PESPacketMustHeaderLength {
		pes_header_data_len := int(d.buffer[8])
		if len(d.buffer) < pes_header_data_len+9 {
			return
		}
		start_code_prerix := uint32(d.buffer[0])<<16 | uint32(d.buffer[1])<<8 | uint32(d.buffer[2])
		stream_id := d.buffer[3]
		packet_len := uint16(ReadLength(d.buffer[4:]))
		data_aligned := d.buffer[6]&0x4 > 0
		pts_dts_flags := d.buffer[7] >> 6
		pts := uint64(0)
		p := 9
		if pts_dts_flags >= 2 {
			pts = uint64(d.buffer[p]) & 0xe << 29
			pts |= uint64(d.buffer[p+1]) << 22
			pts |= uint64(d.buffer[p+2]) & 0xfe << 14
			pts |= uint64(d.buffer[p+3]) << 7
			pts |= uint64(d.buffer[p+4]) >> 1
			p += 5
		}
		header := &PESPacketHeader{
			start_code_prerix,
			stream_id,
			packet_len,
			data_aligned,
			pts_dts_flags,
			pts,
		}
		if d.onHeader != nil {
			d.onHeader(header)
		}
		d.state = pd_header_parsed
		if len(d.buffer) > pes_header_data_len+9 {
			if d.onData != nil {
				d.onData(d.buffer[pes_header_data_len+9:])
			}
		}
	}
}

func (p *PESPacketHeader) GetPTS() (uint64, bool) {
	return p.pts, p.pts_dts_flags >= 2
}
