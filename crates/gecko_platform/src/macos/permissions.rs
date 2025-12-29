//! macOS Audio Capture Permissions
//!
//! Handles requesting permission for:
//! 1. Microphone (AVCaptureDevice) - required for Process Tap API
//! 2. Screen Recording (CGWindow) - required for Process Tap API

use objc2::runtime::{AnyClass, Bool};
use objc2::msg_send;
use block2::RcBlock;
use std::sync::{Arc, Condvar, Mutex};
use tracing::{info, warn, debug};

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}

/// Check if Microphone permission is granted
pub fn has_microphone_permission() -> bool {
    unsafe {
        let cls = match AnyClass::get(c"AVCaptureDevice") {
            Some(c) => c,
            None => return false,
        };

        // AVMediaTypeAudio = "soun"
        let media_type = objc2_foundation::NSString::from_str("soun");
        
        // authorizationStatusForMediaType:
        // Returns: 0=NotDetermined, 1=Restricted, 2=Denied, 3=Authorized
        let status: isize = msg_send![cls, authorizationStatusForMediaType: &*media_type];
        
        info!("Microphone permission status: {}", status);
        
        status == 3
    }
}

/// Request Microphone permission
///
/// This blocks until the user responds to the prompt (or returns immediately if already decided).
/// Returns true if granted.
pub fn request_microphone_permission() -> bool {
    if has_microphone_permission() {
        return true;
    }

    info!("Requesting Microphone permission...");
    
    unsafe {
        if let Some(cls) = AnyClass::get(c"AVCaptureDevice") {
             let media_type = objc2_foundation::NSString::from_str("soun");
             let status: isize = msg_send![cls, authorizationStatusForMediaType: &*media_type];
             info!("Requesting Microphone permission. Current status: {}", status);
        }
    }

    let pair = Arc::new((Mutex::new(false), Condvar::new()));

    unsafe {
        let cls = match AnyClass::get(c"AVCaptureDevice") {
            Some(c) => c,
            None => {
                warn!("AVCaptureDevice class not found");
                return false;
            }
        };

        let media_type = objc2_foundation::NSString::from_str("soun");

        // Create completion handler block
        // We need to capture the result, so let's use another Arc<Mutex<Option<bool>>>
        let result = Arc::new(Mutex::new(None));
        let result_clone = result.clone();
        
        let pair_clone = pair.clone();
        
        // Note: block2::RcBlock::new requires closure to return ()
        // Use objc2::runtime::Bool for correct ABI/Encoding
        let block = RcBlock::new(move |granted: Bool| -> () {
            debug!("Microphone permission callback: {:?}", granted);
            let mut res = result_clone.lock().unwrap();
            *res = Some(granted.is_true());
            
            let (lock, cvar) = &*pair_clone;
            let mut done = lock.lock().unwrap();
            *done = true;
            cvar.notify_one();
        });

        // requestAccessForMediaType:completionHandler:
        // We pass the block as a pointer. block2::RcBlock implements RefEncode for &RcBlock
        let _: () = msg_send![cls, requestAccessForMediaType: &*media_type, completionHandler: &*block];

        // Wait for callback
        let (lock, cvar) = &*pair;
        let mut done = lock.lock().unwrap();
        while !*done {
            done = cvar.wait(done).unwrap();
        }
        
        // Read result
        let res_lock = result.lock().unwrap();
        res_lock.unwrap_or(false)
    }
}
