//! Drives the media source the way the Frame Server does, without registering anything.

use rigrs_vcam::{Activator, HEIGHT, WIDTH};
use windows::core::{Interface, GUID, PROPVARIANT};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

#[test]
fn source_produces_frames() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap();

        let act: IMFActivate = Activator::new().unwrap().into();
        let src: IMFMediaSource = act.ActivateObject().unwrap();
        assert_eq!(src.GetCharacteristics().unwrap() & MFMEDIASOURCE_IS_LIVE.0 as u32, 1);
        let pd = src.CreatePresentationDescriptor().unwrap();
        assert_eq!(pd.GetStreamDescriptorCount().unwrap(), 1);

        let ex: IMFMediaSourceEx = src.cast().unwrap();
        let sa = ex.GetStreamAttributes(0).unwrap();
        assert_eq!(sa.GetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED).unwrap(), 1);

        src.Start(&pd, &GUID::zeroed(), &PROPVARIANT::default()).unwrap();
        let ev = src.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)).unwrap();
        assert_eq!(ev.GetType().unwrap(), MENewStream.0 as u32);
        let v = ev.GetValue().unwrap();
        let unk: windows::core::IUnknown = (&v).try_into().unwrap();
        let stream: IMFMediaStream = unk.cast().unwrap();
        let ev = src.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)).unwrap();
        assert_eq!(ev.GetType().unwrap(), MESourceStarted.0 as u32);
        let ev = stream.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)).unwrap();
        assert_eq!(ev.GetType().unwrap(), MEStreamStarted.0 as u32);

        for _ in 0..3 {
            stream.RequestSample(None).unwrap();
            let ev = stream.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)).unwrap();
            assert_eq!(ev.GetType().unwrap(), MEMediaSample.0 as u32);
            let v = ev.GetValue().unwrap();
            let unk: windows::core::IUnknown = (&v).try_into().unwrap();
            let sample: IMFSample = unk.cast().unwrap();
            assert_eq!(sample.GetTotalLength().unwrap(), WIDTH * HEIGHT * 4);
        }
        src.Stop().unwrap();
        src.Shutdown().unwrap();
        act.ShutdownObject().unwrap();
        println!("virtual camera source OK: {}x{} RGB32", WIDTH, HEIGHT);
    }
}
