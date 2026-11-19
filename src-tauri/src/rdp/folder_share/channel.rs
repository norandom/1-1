//! The drive redirection channel as 1-1 announces it.
//!
//! IronRDP's RDPDR processor advertises its static channel with only the
//! compression option. mstsc announces rdpdr as INITIALIZED | COMPRESS_RDP,
//! and [MS-RDPBCGR] 2.2.1.3.4.1 describes a channel without
//! CHANNEL_OPTION_INITIALIZED as not set up, so this wrapper adds the option.
//! Testing on 2026-11-19 showed it is not what Windows needs to start drive
//! redirection (that is the audio channel, see `rdp::client`); it is kept to
//! match mstsc.

use ironrdp_core::impl_as_any;
use ironrdp_pdu::PduResult;
use ironrdp_pdu::gcc::{ChannelName, ChannelOptions};
use ironrdp_rdpdr::Rdpdr;
use ironrdp_svc::{CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};

/// IronRDP's RDPDR processor, announced as an initialized channel.
#[derive(Debug)]
pub(crate) struct DriveChannel(pub(crate) Rdpdr);

impl_as_any!(DriveChannel);

impl SvcProcessor for DriveChannel {
    fn channel_name(&self) -> ChannelName {
        self.0.channel_name()
    }

    fn compression_condition(&self) -> CompressionCondition {
        self.0.compression_condition()
    }

    fn channel_options(&self) -> ChannelOptions {
        self.0.channel_options() | ChannelOptions::INITIALIZED
    }

    fn start(&mut self) -> PduResult<Vec<SvcMessage>> {
        self.0.start()
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        self.0.process(payload)
    }
}

impl SvcClientProcessor for DriveChannel {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct NoBackend;
    ironrdp_core::impl_as_any!(NoBackend);
    impl ironrdp_rdpdr::backend::RdpdrBackend for NoBackend {
        fn handle_server_device_announce_response(
            &mut self,
            _pdu: ironrdp_rdpdr::pdu::efs::ServerDeviceAnnounceResponse,
        ) -> PduResult<()> {
            Ok(())
        }
        fn handle_scard_call(
            &mut self,
            _req: ironrdp_rdpdr::pdu::efs::DeviceControlRequest<
                ironrdp_rdpdr::pdu::esc::ScardIoCtlCode,
            >,
            _call: ironrdp_rdpdr::pdu::esc::ScardCall,
        ) -> PduResult<Vec<SvcMessage>> {
            Ok(Vec::new())
        }
        fn handle_drive_io_request(
            &mut self,
            _req: ironrdp_rdpdr::pdu::efs::ServerDriveIoRequest,
        ) -> PduResult<Vec<SvcMessage>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn the_drive_channel_is_announced_as_initialized_like_mstsc() {
        let inner = Rdpdr::new(Box::new(NoBackend), "1-1".into());
        let bare = inner.channel_options();
        let channel = DriveChannel(Rdpdr::new(Box::new(NoBackend), "1-1".into()));
        assert!(
            !bare.contains(ChannelOptions::INITIALIZED),
            "IronRDP's own announcement"
        );
        assert_eq!(
            channel.channel_options(),
            ChannelOptions::INITIALIZED | ChannelOptions::COMPRESS_RDP
        );
        assert_eq!(channel.channel_name(), inner.channel_name());
    }
}
