use crate::{RafxApiDef, RafxResult};
use raw_window_handle::{HasRawDisplayHandle, HasRawWindowHandle};
use std::sync::Arc;

use crate::gles2::{RafxDeviceContextGles2, RafxDeviceContextGles2Inner};

/// Gl-specific configuration
#[derive(Default)]
pub struct RafxApiDefGles2 {
    pub validate_shaders: bool,
}

pub struct RafxApiGles2 {
    device_context: Option<RafxDeviceContextGles2>,
}

impl Drop for RafxApiGles2 {
    fn drop(&mut self) {
        self.destroy().unwrap();
    }
}

impl RafxApiGles2 {
    pub fn device_context(&self) -> &RafxDeviceContextGles2 {
        self.device_context.as_ref().unwrap()
    }

    pub fn new(
        display: &dyn HasRawDisplayHandle,
        window: &dyn HasRawWindowHandle,
        _api_def: &RafxApiDef,
        gl_api_def: &RafxApiDefGles2,
    ) -> RafxResult<Self> {
        let inner = Arc::new(RafxDeviceContextGles2Inner::new(
            display, window, gl_api_def,
        )?);
        let device_context = RafxDeviceContextGles2::new(inner)?;

        Ok(RafxApiGles2 {
            device_context: Some(device_context),
        })
    }

    pub fn destroy(&mut self) -> RafxResult<()> {
        if let Some(device_context) = self.device_context.take() {
            // Clear any internal caches that may hold references to the device
            let inner = device_context.inner.clone();

            #[cfg(debug_assertions)]
            #[cfg(feature = "track-device-contexts")]
            let _create_index = device_context.create_index;

            // Thsi should be the final device context
            std::mem::drop(device_context);

            let _strong_count = Arc::strong_count(&inner);
            match Arc::try_unwrap(inner) {
                Ok(inner) => std::mem::drop(inner),
                Err(_arc) => {
                    Err(format!(
                        "Could not destroy device, {} references to it exist",
                        _strong_count
                    ))?;

                    #[cfg(debug_assertions)]
                    #[cfg(feature = "track-device-contexts")]
                    {
                        let mut all_contexts = _arc.all_contexts.lock().unwrap();
                        all_contexts.remove(&_create_index);
                        for (k, v) in all_contexts.iter_mut() {
                            v.resolve();
                            println!("context allocation: {}\n{:?}", k, v);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
