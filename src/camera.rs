//! macOS camera capture (AVFoundation) + face landmarks (Vision).
//!
//! Frames arrive on a private dispatch queue; each one is reduced to its
//! luma plane plus four keypoints and fed straight into `lips::Frontend`,
//! so only the mouth crops outlive the callback.

use std::ffi::c_void;
use std::sync::{Mutex, Once};

use cocoa::base::{id, nil, BOOL, YES};
use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

use crate::lips::{Frontend, Keypoints};

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}
#[link(name = "Vision", kind = "framework")]
extern "C" {}
#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    fn CMSampleBufferGetImageBuffer(sb: *mut c_void) -> *mut c_void;
    fn CMSampleBufferGetPresentationTimeStamp(sb: *mut c_void) -> CMTime;
}
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferLockBaseAddress(pb: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pb: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(pb: *mut c_void, plane: usize) -> *mut u8;
    fn CVPixelBufferGetBytesPerRowOfPlane(pb: *mut c_void, plane: usize) -> usize;
    fn CVPixelBufferGetWidthOfPlane(pb: *mut c_void, plane: usize) -> usize;
    fn CVPixelBufferGetHeightOfPlane(pb: *mut c_void, plane: usize) -> usize;
    fn CVPixelBufferCreate(
        alloc: *mut c_void,
        w: usize,
        h: usize,
        fmt: u32,
        attrs: *mut c_void,
        out: *mut *mut c_void,
    ) -> i32;
    fn CVPixelBufferGetBaseAddress(pb: *mut c_void) -> *mut u8;
    fn CVPixelBufferGetBytesPerRow(pb: *mut c_void) -> usize;
    fn CVPixelBufferRelease(pb: *mut c_void);
}
extern "C" {
    fn dispatch_queue_create(label: *const i8, attr: *mut c_void) -> *mut c_void;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

const LUMA_420F: u32 = u32::from_be_bytes(*b"420f");
const BGRA: u32 = u32::from_be_bytes(*b"BGRA");

/// Live capture state shared with the camera callback.
#[derive(Default)]
pub struct Live {
    pub front: Option<Frontend>,
    pub previewing: bool,
    pub face: bool,
    pub frames: usize,
    pub openness: f32,
    pub started: Option<f64>,
}

pub static LIVE: Mutex<Option<Live>> = Mutex::new(None);

fn ns(s: &str) -> id {
    unsafe { NSString::alloc(nil).init_str(s) }
}

/// 0 = not determined, 1 = restricted, 2 = denied, 3 = authorized.
pub fn permission() -> i64 {
    unsafe { msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: ns("vide")] }
}

pub fn request_permission(done: impl Fn(bool) + Send + 'static) {
    let block = block::ConcreteBlock::new(move |ok: BOOL| done(ok as i64 != 0)).copy();
    unsafe {
        let _: () = msg_send![class!(AVCaptureDevice), requestAccessForMediaType: ns("vide") completionHandler: &*block];
    }
}

pub fn has_camera() -> bool {
    unsafe {
        let d: id = msg_send![class!(AVCaptureDevice), defaultDeviceWithMediaType: ns("vide")];
        d != nil
    }
}

fn delegate_class() -> &'static Class {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let mut decl = ClassDecl::new("ThinSttLipDelegate", class!(NSObject)).unwrap();
        extern "C" fn on_frame(_: &Object, _: Sel, _out: id, sb: id, _conn: id) {
            unsafe { handle_sample(sb as *mut c_void) };
        }
        unsafe {
            decl.add_method(
                sel!(captureOutput:didOutputSampleBuffer:fromConnection:),
                on_frame as extern "C" fn(&Object, Sel, id, id, id),
            );
        }
        decl.register();
    });
    Class::get("ThinSttLipDelegate").unwrap()
}

unsafe fn handle_sample(sb: *mut c_void) {
    let ts = CMSampleBufferGetPresentationTimeStamp(sb);
    let t = ts.value as f64 / ts.timescale.max(1) as f64;
    let mut guard = LIVE.lock().unwrap();
    let Some(live) = guard.as_mut() else { return };
    let t0 = *live.started.get_or_insert(t);
    let t = t - t0;
    let recording = live.front.is_some();
    if recording && !live.front.as_mut().unwrap().wants(t) {
        return;
    }
    if !recording && !live.previewing {
        return;
    }
    let pb = CMSampleBufferGetImageBuffer(sb);
    if pb.is_null() {
        return;
    }
    let marks = detect(pb);
    live.face = marks.is_some();
    if let Some((_, open)) = marks {
        live.openness = open;
    }
    if let Some(front) = live.front.as_mut() {
        CVPixelBufferLockBaseAddress(pb, 1);
        let (w, h) = (CVPixelBufferGetWidthOfPlane(pb, 0), CVPixelBufferGetHeightOfPlane(pb, 0));
        let stride = CVPixelBufferGetBytesPerRowOfPlane(pb, 0);
        let base = CVPixelBufferGetBaseAddressOfPlane(pb, 0);
        let mut luma = Vec::with_capacity(w * h);
        for y in 0..h {
            luma.extend_from_slice(std::slice::from_raw_parts(base.add(y * stride), w));
        }
        CVPixelBufferUnlockBaseAddress(pb, 1);
        let (k, open) = marks.map(|(k, o)| (Some(k), o)).unwrap_or((None, 0.0));
        front.push(t, luma, w, h, k, open);
        live.frames = front.frames();
    }
}

/// Largest face's keypoints in pixel coords (y down) and mouth openness.
pub unsafe fn detect(pb: *mut c_void) -> Option<(Keypoints, f32)> {
    let w = CVPixelBufferGetWidthOfPlane(pb, 0).max(CVPixelBufferGetWidth(pb)) as f64;
    let h = CVPixelBufferGetHeightOfPlane(pb, 0).max(CVPixelBufferGetHeight(pb)) as f64;
    let pool: id = msg_send![class!(NSAutoreleasePool), new];
    let out = (|| {
        let req: id = msg_send![class!(VNDetectFaceLandmarksRequest), new];
        let handler: id = msg_send![class!(VNImageRequestHandler), alloc];
        let dict: id = msg_send![class!(NSDictionary), dictionary];
        let handler: id = msg_send![handler, initWithCVPixelBuffer: pb options: dict];
        let reqs: id = msg_send![class!(NSArray), arrayWithObject: req];
        let ok: BOOL = msg_send![handler, performRequests: reqs error: std::ptr::null_mut::<id>()];
        let results: id = msg_send![req, results];
        let _: () = msg_send![handler, release];
        let res = if ok == YES && results != nil { best_face(results, w, h) } else { None };
        let _: () = msg_send![req, release];
        res
    })();
    let _: () = msg_send![pool, drain];
    out
}

unsafe fn best_face(results: id, w: f64, h: f64) -> Option<(Keypoints, f32)> {
    let n: usize = msg_send![results, count];
    let mut best: Option<(f64, id)> = None;
    for i in 0..n {
        let face: id = msg_send![results, objectAtIndex: i];
        let r: NSRect = msg_send![face, boundingBox];
        let area = r.size.width * r.size.height;
        if best.map_or(true, |(a, _)| area > a) {
            best = Some((area, face));
        }
    }
    let (_, face) = best?;
    let lm: id = msg_send![face, landmarks];
    if lm == nil {
        return None;
    }
    let size = NSSize::new(w, h);
    let pts = |region: id| -> Vec<(f32, f32)> {
        if region == nil {
            return Vec::new();
        }
        let count: usize = msg_send![region, pointCount];
        let p: *const NSPoint = msg_send![region, pointsInImageOfSize: size];
        (0..count)
            .map(|i| {
                let q = *p.add(i);
                (q.x as f32, (h - q.y) as f32)
            })
            .collect()
    };
    let centre = |v: &[(f32, f32)]| -> Option<[f32; 2]> {
        if v.is_empty() {
            return None;
        }
        let n = v.len() as f32;
        Some([v.iter().map(|p| p.0).sum::<f32>() / n, v.iter().map(|p| p.1).sum::<f32>() / n])
    };
    let a = centre(&pts(msg_send![lm, leftEye]))?;
    let b = centre(&pts(msg_send![lm, rightEye]))?;
    let nose = centre(&pts(msg_send![lm, nose]))?;
    let lips = pts(msg_send![lm, outerLips]);
    let mouth = centre(&lips)?;
    let (lo, hi) = lips.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    let (l, r) = lips.iter().fold((f32::MAX, f32::MIN), |(l, r), p| (l.min(p.0), r.max(p.0)));
    let open = ((hi - lo) / (r - l).max(1.0)).clamp(0.0, 1.0);
    // Reference order is image-left eye first (the subject's right eye).
    let (first, second) = if a[0] <= b[0] { (a, b) } else { (b, a) };
    Some(([first, second, nose, mouth], open))
}

extern "C" {
    fn CVPixelBufferGetWidth(pb: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(pb: *mut c_void) -> usize;
}

/// Running camera session. Dropping it stops capture and frees the
/// session, so the camera light goes off as soon as it is not needed.
pub struct Camera {
    session: id,
    delegate: id,
    output: id,
}

unsafe impl Send for Camera {}

impl Camera {
    pub fn start() -> Result<Camera, String> {
        unsafe {
            let dev: id = msg_send![class!(AVCaptureDevice), defaultDeviceWithMediaType: ns("vide")];
            if dev == nil {
                return Err("no camera found".into());
            }
            let mut err: id = nil;
            let input: id = msg_send![class!(AVCaptureDeviceInput), deviceInputWithDevice: dev error: &mut err];
            if input == nil {
                return Err("camera unavailable — allow Camera access in System Settings".into());
            }
            let session: id = msg_send![class!(AVCaptureSession), new];
            let _: () = msg_send![session, setSessionPreset: ns("AVCaptureSessionPreset640x480")];
            let _: () = msg_send![session, addInput: input];
            let output: id = msg_send![class!(AVCaptureVideoDataOutput), new];
            let fmt: id = msg_send![class!(NSNumber), numberWithUnsignedInt: LUMA_420F];
            let key = ns("PixelFormatType");
            let settings: id = msg_send![class!(NSDictionary), dictionaryWithObject: fmt forKey: key];
            let _: () = msg_send![output, setVideoSettings: settings];
            let _: () = msg_send![output, setAlwaysDiscardsLateVideoFrames: YES];
            let delegate: id = msg_send![delegate_class(), new];
            static mut QUEUE: usize = 0;
            if QUEUE == 0 {
                QUEUE = dispatch_queue_create(b"thin-stt.camera\0".as_ptr() as *const i8, std::ptr::null_mut()) as usize;
            }
            let _: () = msg_send![output, setSampleBufferDelegate: delegate queue: QUEUE as *mut c_void];
            let _: () = msg_send![session, addOutput: output];
            let _: () = msg_send![session, startRunning];
            Ok(Camera { session, delegate, output })
        }
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![self.session, stopRunning];
            let _: () = msg_send![self.output, setSampleBufferDelegate: nil queue: std::ptr::null_mut::<c_void>()];
            let _: () = msg_send![self.session, release];
            let _: () = msg_send![self.output, release];
            let _: () = msg_send![self.delegate, release];
        }
    }
}

/// Run one luma frame through Vision (used by the `--lipfile` self-test).
pub fn detect_gray(luma: &[u8], w: usize, h: usize) -> Option<(Keypoints, f32)> {
    unsafe {
        let mut pb: *mut c_void = std::ptr::null_mut();
        if CVPixelBufferCreate(std::ptr::null_mut(), w, h, BGRA, std::ptr::null_mut(), &mut pb) != 0 {
            return None;
        }
        CVPixelBufferLockBaseAddress(pb, 0);
        let base = CVPixelBufferGetBaseAddress(pb);
        let stride = CVPixelBufferGetBytesPerRow(pb);
        for y in 0..h {
            for x in 0..w {
                let v = luma[y * w + x];
                let p = base.add(y * stride + x * 4);
                *p = v;
                *p.add(1) = v;
                *p.add(2) = v;
                *p.add(3) = 255;
            }
        }
        CVPixelBufferUnlockBaseAddress(pb, 0);
        let r = detect(pb);
        CVPixelBufferRelease(pb);
        r
    }
}
