use crate::pes;

pub fn get_pts(pes: &pes::PESPacket) -> Option<u64> {
    if let pes::PESPacketBody::NormalPESPacketBody(ref body) = pes.body {
        return body.pts;
    }
    return None;
}
