use log::{info, trace};

use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio_channel::mpsc::Receiver;

use crate::pes;
use crate::psi;
use crate::ts;

use crate::ts::demuxer::{Register, RegistrationError};

pub trait Spawner: Clone {
    fn spawn(
        &self,
        si: &psi::StreamInfo,
        demux_register: &mut Register,
    ) -> Result<(), RegistrationError>;
}

pub fn spawn_stream_splitter<Sp>(spawner: Sp, mut demux_register: ts::demuxer::Register)
where
    Sp: Spawner + Send + 'static,
{
    let rx = demux_register.try_register(ts::PAT_PID).unwrap();
    tokio::spawn(pat_processor(demux_register, spawner, rx));
}

fn pat_processor<Sp>(
    mut demux_register: ts::demuxer::Register,
    spawner: Sp,
    rx: Receiver<ts::TSPacket>,
) -> impl Future<Item = (), Error = ()>
where
    Sp: Spawner + Send + 'static,
{
    psi::Buffer::new(rx)
        .for_each(move |bytes| {
            let bytes = &bytes[..];
            let table_id = bytes[0];
            if table_id == psi::PROGRAM_ASSOCIATION_SECTION {
                let pas = match psi::ProgramAssociationSection::parse(bytes) {
                    Ok(pas) => pas,
                    Err(e) => {
                        info!("err {}: {:#?}", line!(), e);
                        return Ok(());
                    }
                };
                for (program_number, pid) in pas.program_association {
                    if program_number != 0 {
                        // not network pid
                        match demux_register.try_register(pid) {
                            Ok(rx) => {
                                tokio::spawn(pmt_processor(
                                    demux_register.clone(),
                                    spawner.clone(),
                                    rx,
                                ));
                            }
                            Err(e) => {
                                if e.is_closed() {
                                    return Err(e.into());
                                }
                            }
                        }
                    }
                }
            }
            Ok(())
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

fn pmt_processor<Sp>(
    mut demux_regiser: ts::demuxer::Register,
    spawner: Sp,
    rx: Receiver<ts::TSPacket>,
) -> impl Future<Item = (), Error = ()>
where
    Sp: Spawner,
{
    psi::Buffer::new(rx)
        .for_each(move |bytes| {
            let bytes = &bytes[..];
            let table_id = bytes[0];
            if table_id == psi::TS_PROGRAM_MAP_SECTION {
                let pms = match psi::TSProgramMapSection::parse(bytes) {
                    Ok(pms) => pms,
                    Err(e) => {
                        info!("err {}: {:#?}", line!(), e);
                        return Ok(());
                    }
                };
                trace!("program map section: {:#?}", pms);
                for si in pms.stream_info.iter() {
                    if let Err(e) = spawner.spawn(&si, &mut demux_regiser) {
                        if e.is_closed() {
                            return Err(e.into());
                        }
                    }
                }
            }
            Ok(())
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

pub fn get_pts(pes: &pes::PESPacket) -> Option<u64> {
    if let pes::PESPacketBody::NormalPESPacketBody(ref body) = pes.body {
        return body.pts;
    }
    return None;
}
