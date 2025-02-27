use super::gles3_bindings;
use super::gles3_bindings::types::GLenum;
use super::gles3_bindings::Gles3;
use super::WindowHash;
use crate::gles3::gles3_bindings::types::{GLboolean, GLint};
use crate::gles3::{
    ActiveUniformInfo, BufferId, FramebufferId, ProgramId, RenderbufferId, ShaderId, TextureId,
};
use crate::internal_shared::gl_window;
use crate::{RafxError, RafxResult};
use fnv::FnvHashSet;
use raw_window_handle::{HasRawDisplayHandle, HasRawWindowHandle};
use std::ffi::{CStr, CString};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocationId(u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UniformBlockIndex(u32);

pub struct GetActiveUniformMaxNameLengthHint(i32);

extern "system" fn debug_callback(
    source: gles3_bindings::types::GLenum,
    ty: gles3_bindings::types::GLenum,
    _id: gles3_bindings::types::GLuint,
    severity: gles3_bindings::types::GLenum,
    _length: gles3_bindings::types::GLsizei,
    message: *const gles3_bindings::types::GLchar,
    _user_param: *mut std::ffi::c_void,
) {
    use gles3_bindings as gl;

    let message =
        unsafe { String::from_utf8(CStr::from_ptr(message).to_bytes().to_vec()).unwrap() };

    let severity = match severity {
        gl::DEBUG_SEVERITY_NOTIFICATION => log::Level::Info,
        gl::DEBUG_SEVERITY_LOW => log::Level::Info,
        gl::DEBUG_SEVERITY_MEDIUM => log::Level::Warn,
        gl::DEBUG_SEVERITY_HIGH => log::Level::Error,
        _ => unimplemented!(),
    };

    let source = match source {
        gl::DEBUG_SOURCE_API => "Api",
        gl::DEBUG_SOURCE_WINDOW_SYSTEM => "Window System",
        gl::DEBUG_SOURCE_SHADER_COMPILER => "Shader Compiler",
        gl::DEBUG_SOURCE_THIRD_PARTY => "Third Party",
        gl::DEBUG_SOURCE_APPLICATION => "Application",
        gl::DEBUG_SOURCE_OTHER => "Other",
        _ => unimplemented!(),
    };

    let ty = match ty {
        gl::DEBUG_TYPE_ERROR => "Error",
        gl::DEBUG_TYPE_DEPRECATED_BEHAVIOR => "DeprecatedBehavior",
        gl::DEBUG_TYPE_UNDEFINED_BEHAVIOR => "UndefinedBehavior",
        gl::DEBUG_TYPE_PORTABILITY => "Portability",
        gl::DEBUG_TYPE_PERFORMANCE => "Performance",
        gl::DEBUG_TYPE_MARKER => "Marker",
        gl::DEBUG_TYPE_PUSH_GROUP => "PushGroup",
        gl::DEBUG_TYPE_POP_GROUP => "PopGroup",
        gl::DEBUG_TYPE_OTHER => "Other",
        _ => "Unknown",
    };

    log::log!(
        severity,
        "[GL DEBUG OUTPUT][{}][{}]: {}",
        source,
        ty,
        message
    );
}

pub struct GlContext {
    context: gl_window::GlContext,
    gles3: Gles3,
    window_hash: WindowHash,
    extensions: FnvHashSet<String>,

    // GL ES 2.0 does not support VAO, but desktop GL core profile *requires* one to be bound. So
    // we bind a single global VAO at startup if the APIs to do so are available. This allows
    // downstream code to always act as though VAOs are not supported at all
    global_vao: u32,
}

unsafe impl Send for GlContext {}
unsafe impl Sync for GlContext {}

impl PartialEq for GlContext {
    fn eq(
        &self,
        other: &Self,
    ) -> bool {
        self.window_hash == other.window_hash
    }
}

impl Drop for GlContext {
    fn drop(&mut self) {
        if self.gles3.GenVertexArrays.is_loaded() {
            unsafe {
                self.gles3.BindVertexArray(0);
                self.check_for_error().unwrap();
                self.gles3.DeleteVertexArrays(1, &self.global_vao);
                self.check_for_error().unwrap();
            }
        }
    }
}

impl GlContext {
    pub fn new(
        display: &dyn HasRawDisplayHandle,
        window: &dyn HasRawWindowHandle,
        share: Option<&GlContext>,
    ) -> RafxResult<Self> {
        let window_hash = super::calculate_window_hash(display, window);

        let enable_debug = true;

        let mut config = gl_window::GlConfig::default();
        config.profile = gl_window::Profile::Core;
        config.version = (3, 2);
        config.use_debug_context = enable_debug;

        let context =
            gl_window::GlContext::create(display, window, config, share.map(|x| x.context()))
                .unwrap();
        context.make_current();
        let gles3 = Gles3::load_with(|symbol| context.get_proc_address(symbol) as *const _);

        // The correct way to get extensions differs between platforms:
        // GL ES 2.0: gl_get_string(gles3_bindings::EXTENSIONS) returns a space-delimited string
        // Desktop: gl_get_integerv(gles3_bindings::NUM_EXTENSIONS), then call
        //          gl_get_stringi(gles3_bindings::EXTENSIONS, i)
        // GL ES 3.0: I think either work fine
        let mut extensions = FnvHashSet::default();
        if gles3.GetStringi.is_loaded() {
            // GL ES 3.0/Desktop path. Bindings are 2.0 only, so specify the constant for this here
            const NUM_EXTENSIONS: u32 = 0x821D;
            let extension_count = gl_get_integerv(&gles3, NUM_EXTENSIONS) as u32;
            for i in 0..extension_count {
                let extension = gl_get_stringi(&gles3, gles3_bindings::EXTENSIONS, i);
                log::debug!("Extension: {}", extension);
                extensions.insert(extension);
            }
        } else {
            // GL ES 2.0 Path
            let extensions_str = gl_get_string(&gles3, gles3_bindings::EXTENSIONS);
            for extension in extensions_str.split(" ") {
                log::debug!("Extension: {}", extension);
                extensions.insert(extension.to_string());
            }
        }

        // Desktop-only debug functionality
        if enable_debug && gles3.DebugMessageCallback.is_loaded() {
            unsafe {
                gles3.Enable(gles3_bindings::DEBUG_OUTPUT);
                gles3.Enable(gles3_bindings::DEBUG_OUTPUT_SYNCHRONOUS);
                gles3.DebugMessageCallback(Some(debug_callback), std::ptr::null());

                // Enable all messages
                gles3.DebugMessageControl(
                    gles3_bindings::DONT_CARE,
                    gles3_bindings::DONT_CARE,
                    gles3_bindings::DONT_CARE,
                    0,
                    std::ptr::null(),
                    gles3_bindings::TRUE,
                );

                // Disable notifications, they can be very noisy
                gles3.DebugMessageControl(
                    gles3_bindings::DONT_CARE,
                    gles3_bindings::DONT_CARE,
                    gles3_bindings::DEBUG_SEVERITY_NOTIFICATION,
                    0,
                    std::ptr::null(),
                    gles3_bindings::FALSE,
                );
            }
        }

        // Create a "default" VAO that we will use everywhere. This is not supported in ES 2.0 and a
        // single global VAO is used instead. Desktop GL *requires* VAOs to be created explicitly
        let mut global_vao = 0;
        if gles3.GenVertexArrays.is_loaded() {
            unsafe {
                gles3.GenVertexArrays(1, &mut global_vao);
                check_for_error(&gles3)?;
                gles3.BindVertexArray(global_vao);
                check_for_error(&gles3)?;
            }
        }

        context.make_not_current();

        Ok(GlContext {
            context,
            gles3,
            window_hash,
            extensions,
            global_vao,
        })
    }

    pub fn window_hash(&self) -> WindowHash {
        self.window_hash
    }

    pub fn context(&self) -> &gl_window::GlContext {
        &self.context
    }

    pub fn gles3(&self) -> &Gles3 {
        &self.gles3
    }

    pub fn make_current(&self) {
        self.context.make_current();
        unsafe {
            self.gles3.BindVertexArray(self.global_vao);
            self.check_for_error().unwrap();
        }
    }

    pub fn make_not_current(&self) {
        unsafe {
            self.gles3.BindVertexArray(0);
            self.check_for_error().unwrap();
        }
        self.context.make_not_current();
    }

    pub fn swap_buffers(&self) {
        self.context.swap_buffers();
    }

    pub fn check_for_error(&self) -> RafxResult<()> {
        check_for_error(&self.gles3)
    }

    pub fn has_extension(
        &self,
        name: &str,
    ) -> bool {
        self.extensions.contains(name)
    }

    pub fn gl_get_integerv(
        &self,
        pname: u32,
    ) -> i32 {
        gl_get_integerv(&self.gles3, pname)
    }

    pub fn gl_get_string(
        &self,
        pname: u32,
    ) -> String {
        gl_get_string(&self.gles3, pname)
    }

    pub fn gl_get_stringi(
        &self,
        pname: u32,
        index: u32,
    ) -> String {
        unsafe {
            let str = self.gles3.GetStringi(pname, index);
            if str.is_null() {
                return "".to_string();
            }

            std::ffi::CStr::from_ptr(str as _)
                .to_str()
                .unwrap()
                .to_string()
        }
    }

    pub fn gl_viewport(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.Viewport(x, y, width, height);
            self.check_for_error()
        }
    }

    pub fn gl_scissor(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.Scissor(x, y, width, height);
            self.check_for_error()
        }
    }

    pub fn gl_depth_rangef(
        &self,
        n: f32,
        f: f32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.DepthRangef(n, f);
            self.check_for_error()
        }
    }

    pub fn gl_clear_color(
        &self,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.ClearColor(r, g, b, a);
            self.check_for_error()
        }
    }

    pub fn gl_clear_depthf(
        &self,
        d: f32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.ClearDepthf(d);
            self.check_for_error()
        }
    }

    pub fn gl_clear_stencil(
        &self,
        s: i32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.ClearStencil(s);
            self.check_for_error()
        }
    }

    pub fn gl_clear(
        &self,
        mask: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.Clear(mask);
            self.check_for_error()
        }
    }

    pub fn gl_finish(&self) -> RafxResult<()> {
        unsafe {
            self.gles3.Finish();
            self.check_for_error()
        }
    }

    pub fn gl_create_framebuffer(&self) -> RafxResult<FramebufferId> {
        unsafe {
            let mut framebuffer = 0;
            self.gles3.GenFramebuffers(1, &mut framebuffer);
            self.check_for_error()?;
            Ok(FramebufferId(framebuffer))
        }
    }

    pub fn gl_destroy_framebuffer(
        &self,
        framebuffer_id: FramebufferId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.DeleteFramebuffers(1, &framebuffer_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_create_texture(&self) -> RafxResult<TextureId> {
        unsafe {
            let mut texture = 0;
            self.gles3.GenTextures(1, &mut texture);
            self.check_for_error()?;
            Ok(TextureId(texture))
        }
    }

    pub fn gl_destroy_texture(
        &self,
        texture_id: TextureId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.DeleteTextures(1, &texture_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_create_buffer(&self) -> RafxResult<BufferId> {
        unsafe {
            let mut buffer = 0;
            self.gles3.GenBuffers(1, &mut buffer);
            self.check_for_error()?;
            Ok(BufferId(buffer))
        }
    }

    pub fn gl_destroy_buffer(
        &self,
        buffer_id: BufferId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.DeleteBuffers(1, &buffer_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_bind_buffer(
        &self,
        target: GLenum,
        buffer_id: BufferId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.BindBuffer(target, buffer_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_bind_framebuffer(
        &self,
        target: GLenum,
        framebuffer_id: FramebufferId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.BindFramebuffer(target, framebuffer_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_vertex_attrib_pointer(
        &self,
        index: u32,
        size: i32,
        type_: GLenum,
        normalized: bool,
        stride: u32,
        byte_offset: i32,
    ) -> RafxResult<()> {
        unsafe {
            let ptr = byte_offset as *const std::ffi::c_void;
            self.gles3.VertexAttribPointer(
                index,
                size,
                type_,
                to_gl_bool(normalized),
                stride as _,
                ptr,
            );
            self.check_for_error()
        }
    }

    pub fn gl_vertex_attrib_divisor(
        &self,
        index: u32,
        divisor: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.VertexAttribDivisor(index, divisor);
            self.check_for_error()
        }
    }

    pub fn gl_enable_vertex_attrib_array(
        &self,
        index: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.EnableVertexAttribArray(index);
            self.check_for_error()
        }
    }

    pub fn gl_buffer_data(
        &self,
        target: GLenum,
        size: u64,
        data: *const std::ffi::c_void,
        usage: GLenum,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.BufferData(target, size as _, data, usage);
            self.check_for_error()
        }
    }

    pub fn gl_buffer_sub_data(
        &self,
        target: GLenum,
        offset: u32,
        size: u64,
        data: *const u8,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .BufferSubData(target, offset as _, size as _, data as _);
            self.check_for_error()
        }
    }

    pub fn gl_create_shader(
        &self,
        shader_type: GLenum,
    ) -> RafxResult<ShaderId> {
        unsafe {
            let id = self.gles3.CreateShader(shader_type);
            self.check_for_error()?;
            Ok(ShaderId(id))
        }
    }

    pub fn gl_destroy_shader(
        &self,
        shader_id: ShaderId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.DeleteShader(shader_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_shader_source(
        &self,
        shader_id: ShaderId,
        code: &CString,
    ) -> RafxResult<()> {
        unsafe {
            let len: GLint = code.as_bytes().len() as _;
            self.gles3
                .ShaderSource(shader_id.0, 1, &code.as_ptr(), &len);
            self.check_for_error()
        }
    }

    pub fn gl_compile_shader(
        &self,
        shader_id: ShaderId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.CompileShader(shader_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_get_shaderiv(
        &self,
        shader_id: ShaderId,
        pname: GLenum,
    ) -> RafxResult<i32> {
        unsafe {
            let mut value = 0;
            self.gles3.GetShaderiv(shader_id.0, pname, &mut value);
            self.check_for_error()?;
            Ok(value)
        }
    }

    pub fn gl_get_programiv(
        &self,
        program_id: ProgramId,
        pname: GLenum,
    ) -> RafxResult<i32> {
        unsafe {
            let mut value = 0;
            self.gles3.GetProgramiv(program_id.0, pname, &mut value);
            self.check_for_error()?;
            Ok(value)
        }
    }

    fn gl_get_shader_info_log(
        &self,
        shader_id: ShaderId,
        string: &mut [u8],
    ) -> RafxResult<()> {
        unsafe {
            let len = string.len();
            self.gles3.GetShaderInfoLog(
                shader_id.0,
                len as _,
                std::ptr::null_mut(),
                string.as_mut_ptr() as _,
            );
            self.check_for_error()
        }
    }

    fn gl_get_program_info_log(
        &self,
        program_id: ProgramId,
        string: &mut [u8],
    ) -> RafxResult<()> {
        unsafe {
            let len = string.len();
            self.gles3.GetProgramInfoLog(
                program_id.0,
                len as _,
                std::ptr::null_mut(),
                string.as_mut_ptr() as _,
            );
            self.check_for_error()
        }
    }

    pub fn get_shader_info_log(
        &self,
        shader_id: ShaderId,
    ) -> RafxResult<Option<String>> {
        let error_len = self.gl_get_shaderiv(shader_id, gles3_bindings::INFO_LOG_LENGTH)?;
        if error_len == 0 {
            return Ok(None);
        };

        let mut log = vec![0_u8; error_len as usize];
        self.gl_get_shader_info_log(shader_id, &mut log)?;
        Ok(Some(String::from_utf8(log).unwrap()))
    }

    pub fn compile_shader(
        &self,
        shader_type: GLenum,
        src: &CString,
    ) -> RafxResult<ShaderId> {
        let shader_id = self.gl_create_shader(shader_type)?;
        self.gl_shader_source(shader_id, &src)?;
        self.gl_compile_shader(shader_id)?;
        if self.gl_get_shaderiv(shader_id, gles3_bindings::COMPILE_STATUS)? == 0 {
            return Err(match self.get_shader_info_log(shader_id)? {
                Some(x) => format!("Error compiling shader: {}", x),
                None => "Error compiling shader, info log not available".to_string(),
            })?;
        }

        if let Ok(Some(debug_info)) = self.get_shader_info_log(shader_id) {
            log::debug!("Debug info while compiling shader program: {}", debug_info);
        }

        Ok(shader_id)
    }

    pub fn gl_create_program(&self) -> RafxResult<ProgramId> {
        unsafe {
            let program_id = self.gles3.CreateProgram();
            self.check_for_error()?;
            Ok(ProgramId(program_id))
        }
    }

    pub fn gl_destroy_program(
        &self,
        program_id: ProgramId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.DeleteProgram(program_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_attach_shader(
        &self,
        program_id: ProgramId,
        shader_id: ShaderId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.AttachShader(program_id.0, shader_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_link_program(
        &self,
        program_id: ProgramId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.LinkProgram(program_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_validate_program(
        &self,
        program_id: ProgramId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.ValidateProgram(program_id.0);
            self.check_for_error()
        }
    }

    fn get_program_info_log(
        &self,
        program_id: ProgramId,
    ) -> RafxResult<Option<String>> {
        let error_len = self.gl_get_programiv(program_id, gles3_bindings::INFO_LOG_LENGTH)?;
        if error_len == 0 {
            return Ok(None);
        };

        let mut log = vec![0_u8; error_len as usize];
        self.gl_get_program_info_log(program_id, &mut log)?;
        Ok(Some(String::from_utf8(log).unwrap()))
    }

    pub fn link_shader_program(
        &self,
        program_id: ProgramId,
    ) -> RafxResult<()> {
        self.gl_link_program(program_id)?;
        if self.gl_get_programiv(program_id, gles3_bindings::LINK_STATUS)? == 0 {
            return Err(match self.get_program_info_log(program_id)? {
                Some(x) => format!("Error linking shader program: {}", x),
                None => "Error linking shader program, info log not available".to_string(),
            })?;
        }

        if let Ok(Some(debug_info)) = self.get_program_info_log(program_id) {
            log::debug!("Debug info while linking shader program: {}", debug_info);
        }

        Ok(())
    }

    pub fn validate_shader_program(
        &self,
        program_id: ProgramId,
    ) -> RafxResult<()> {
        self.gl_validate_program(program_id)?;
        if self.gl_get_programiv(program_id, gles3_bindings::VALIDATE_STATUS)? == 0 {
            return Err(match self.get_program_info_log(program_id)? {
                Some(x) => format!("Error validating shader program: {}", x),
                None => "Error validating shader program, info log not available".to_string(),
            })?;
        }

        if let Ok(Some(debug_info)) = self.get_program_info_log(program_id) {
            log::debug!("Debug info while validating shader program: {}", debug_info);
        }

        Ok(())
    }

    pub fn gl_get_uniform_block_index(
        &self,
        program_id: ProgramId,
        block_name: &CStr,
    ) -> RafxResult<Option<UniformBlockIndex>> {
        unsafe {
            let value = self
                .gles3
                .GetUniformBlockIndex(program_id.0, block_name.as_ptr());
            self.check_for_error()?;

            if value == gles3_bindings::INVALID_INDEX {
                return Ok(None);
            }

            Ok(Some(UniformBlockIndex(value as u32)))
        }
    }

    pub fn gl_get_active_uniform_blockiv(
        &self,
        program_id: ProgramId,
        block_index: UniformBlockIndex,
        pname: GLenum,
    ) -> RafxResult<i32> {
        unsafe {
            let mut value = 0;
            self.gles3
                .GetActiveUniformBlockiv(program_id.0, block_index.0, pname, &mut value);
            self.check_for_error()?;
            Ok(value)
        }
    }

    pub fn gl_uniform_block_binding(
        &self,
        program_id: ProgramId,
        block_index: UniformBlockIndex,
        binding: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .UniformBlockBinding(program_id.0, block_index.0, binding);
            self.check_for_error()
        }
    }

    pub fn gl_bind_buffer_base(
        &self,
        target: GLenum,
        binding: u32,
        buffer: BufferId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.BindBufferBase(target, binding, buffer.0);
            self.check_for_error()
        }
    }

    pub fn gl_bind_buffer_range(
        &self,
        target: GLenum,
        binding: u32,
        buffer: BufferId,
        offset: u64,
        size: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .BindBufferRange(target, binding, buffer.0, offset as _, size as _);
            self.check_for_error()
        }
    }

    pub fn gl_get_uniform_location(
        &self,
        program_id: ProgramId,
        name: &CStr,
    ) -> RafxResult<Option<LocationId>> {
        unsafe {
            let value = self.gles3.GetUniformLocation(program_id.0, name.as_ptr());
            self.check_for_error()?;

            if value == -1 {
                return Ok(None);
            }

            Ok(Some(LocationId(value as u32)))
        }
    }

    pub fn get_active_uniform_max_name_length_hint(
        &self,
        program_id: ProgramId,
    ) -> RafxResult<GetActiveUniformMaxNameLengthHint> {
        let max_length =
            self.gl_get_programiv(program_id, gles3_bindings::ACTIVE_UNIFORM_MAX_LENGTH)?;
        Ok(GetActiveUniformMaxNameLengthHint(max_length))
    }

    pub fn gl_get_active_uniform(
        &self,
        program_id: ProgramId,
        index: u32,
        max_name_length_hint: &GetActiveUniformMaxNameLengthHint,
    ) -> RafxResult<ActiveUniformInfo> {
        let mut name_length = 0;
        let mut size = 0;
        let mut ty = 0;
        let mut name_buffer = vec![0_u8; max_name_length_hint.0 as usize];

        unsafe {
            self.gles3.GetActiveUniform(
                program_id.0,
                index,
                max_name_length_hint.0 as _,
                &mut name_length,
                &mut size,
                &mut ty,
                name_buffer.as_mut_ptr() as _,
            );
        }
        self.check_for_error()?;

        let name = CString::new(&name_buffer[0..name_length as usize]).unwrap();

        Ok(ActiveUniformInfo {
            name,
            size: size as u32,
            ty,
        })
    }

    pub fn gl_flush(&self) -> RafxResult<()> {
        unsafe {
            self.gles3.Flush();
            self.check_for_error()
        }
    }

    pub fn gl_disable(
        &self,
        value: GLenum,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.Disable(value);
            self.check_for_error()
        }
    }

    pub fn gl_enable(
        &self,
        value: GLenum,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.Enable(value);
            self.check_for_error()
        }
    }

    pub fn gl_cull_face(
        &self,
        mode: GLenum,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.CullFace(mode);
            self.check_for_error()
        }
    }

    pub fn gl_front_face(
        &self,
        mode: GLenum,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.FrontFace(mode);
            self.check_for_error()
        }
    }

    pub fn gl_depth_mask(
        &self,
        flag: bool,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.DepthMask(to_gl_bool(flag));
            self.check_for_error()
        }
    }

    pub fn gl_depth_func(
        &self,
        value: GLenum,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.DepthFunc(value);
            self.check_for_error()
        }
    }

    pub fn gl_stencil_mask(
        &self,
        mask: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.StencilMask(mask);
            self.check_for_error()
        }
    }

    pub fn gl_stencil_func_separate(
        &self,
        face: GLenum,
        func: GLenum,
        ref_value: i32,
        mask: GLenum,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.StencilFuncSeparate(face, func, ref_value, mask);
            self.check_for_error()
        }
    }

    pub fn gl_stencil_op_separate(
        &self,
        face: GLenum,
        sfail: GLenum,
        dpfail: GLenum,
        dppass: GLenum,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.StencilOpSeparate(face, sfail, dpfail, dppass);
            self.check_for_error()
        }
    }

    pub fn gl_blend_func_separate(
        &self,
        sfactor_rgb: GLenum,
        dfactor_rgb: GLenum,
        sfactor_alpha: GLenum,
        dfactor_alpha: GLenum,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .BlendFuncSeparate(sfactor_rgb, dfactor_rgb, sfactor_alpha, dfactor_alpha);
            self.check_for_error()
        }
    }

    pub fn gl_blend_equation_separate(
        &self,
        mode_rgb: GLenum,
        mode_alpha: GLenum,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.BlendEquationSeparate(mode_rgb, mode_alpha);
            self.check_for_error()
        }
    }

    pub fn gl_color_mask(
        &self,
        r: bool,
        g: bool,
        b: bool,
        a: bool,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .ColorMask(to_gl_bool(r), to_gl_bool(g), to_gl_bool(b), to_gl_bool(a));
            self.check_for_error()
        }
    }

    pub fn gl_bind_attrib_location(
        &self,
        program_id: ProgramId,
        index: u32,
        name: &str,
    ) -> RafxResult<()> {
        unsafe {
            let cstr = CString::new(name).unwrap();
            self.gles3
                .BindAttribLocation(program_id.0, index, cstr.as_ptr());
            self.check_for_error()
        }
    }

    pub fn gl_use_program(
        &self,
        program_id: ProgramId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.UseProgram(program_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_bind_renderbuffer(
        &self,
        target: GLenum,
        renderbuffer: RenderbufferId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.BindRenderbuffer(target, renderbuffer.0);
            self.check_for_error()
        }
    }

    pub fn gl_framebuffer_renderbuffer(
        &self,
        target: GLenum,
        attachment: GLenum,
        renderbuffer_target: GLenum,
        renderbuffer: RenderbufferId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.FramebufferRenderbuffer(
                target,
                attachment,
                renderbuffer_target,
                renderbuffer.0,
            );
            self.check_for_error()
        }
    }

    pub fn gl_framebuffer_texture(
        &self,
        target: GLenum,
        attachment: GLenum,
        texture_target: GLenum,
        texture_id: TextureId,
        mip_level: u8,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.FramebufferTexture2D(
                target,
                attachment,
                texture_target,
                texture_id.0,
                mip_level as _,
            );
            self.check_for_error()
        }
    }

    pub fn gl_check_framebuffer_status(
        &self,
        target: GLenum,
    ) -> RafxResult<u32> {
        unsafe {
            let result = self.gles3.CheckFramebufferStatus(target);
            self.check_for_error()?;
            Ok(result)
        }
    }

    pub fn gl_disable_vertex_attrib_array(
        &self,
        index: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.DisableVertexAttribArray(index);
            self.check_for_error()
        }
    }

    pub fn gl_draw_arrays(
        &self,
        mode: GLenum,
        first: i32,
        count: i32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.DrawArrays(mode, first, count);
            self.check_for_error()
        }
    }

    pub fn gl_draw_elements(
        &self,
        mode: GLenum,
        count: i32,
        type_: GLenum,
        byte_offset: u32,
    ) -> RafxResult<()> {
        unsafe {
            let ptr = byte_offset as *const std::ffi::c_void;
            self.gles3.DrawElements(mode, count, type_, ptr);
            self.check_for_error()
        }
    }

    pub fn gl_uniform_1iv<T: Copy>(
        &self,
        location: &LocationId,
        data: &T,
        count: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .Uniform1iv(location.0 as _, count as _, data as *const T as _);
            self.check_for_error()
        }
    }

    pub fn gl_uniform_1fv<T: Copy>(
        &self,
        location: &LocationId,
        data: &T,
        count: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .Uniform1fv(location.0 as _, count as _, data as *const T as _);
            self.check_for_error()
        }
    }

    pub fn gl_uniform_2iv<T: Copy>(
        &self,
        location: &LocationId,
        data: &T,
        count: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .Uniform2iv(location.0 as _, count as _, data as *const T as _);
            self.check_for_error()
        }
    }

    pub fn gl_uniform_2fv<T: Copy>(
        &self,
        location: &LocationId,
        data: &T,
        count: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .Uniform2fv(location.0 as _, count as _, data as *const T as _);
            self.check_for_error()
        }
    }

    pub fn gl_uniform_3iv<T: Copy>(
        &self,
        location: &LocationId,
        data: &T,
        count: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .Uniform3iv(location.0 as _, count as _, data as *const T as _);
            self.check_for_error()
        }
    }

    pub fn gl_uniform_3fv<T: Copy>(
        &self,
        location: &LocationId,
        data: &T,
        count: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .Uniform3fv(location.0 as _, count as _, data as *const T as _);
            self.check_for_error()
        }
    }

    pub fn gl_uniform_4iv<T: Copy>(
        &self,
        location: &LocationId,
        data: &T,
        count: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .Uniform4iv(location.0 as _, count as _, data as *const T as _);
            self.check_for_error()
        }
    }

    pub fn gl_uniform_4fv<T: Copy>(
        &self,
        location: &LocationId,
        data: &T,
        count: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3
                .Uniform4fv(location.0 as _, count as _, data as *const T as _);
            self.check_for_error()
        }
    }

    pub fn gl_uniform_matrix_2fv<T: Copy>(
        &self,
        location: &LocationId,
        data: &T,
        count: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.UniformMatrix2fv(
                location.0 as _,
                count as _,
                gles3_bindings::FALSE,
                data as *const T as _,
            );
            self.check_for_error()
        }
    }

    pub fn gl_uniform_matrix_3fv<T: Copy>(
        &self,
        location: &LocationId,
        data: &T,
        count: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.UniformMatrix3fv(
                location.0 as _,
                count as _,
                gles3_bindings::FALSE,
                data as *const T as _,
            );
            self.check_for_error()
        }
    }

    pub fn gl_uniform_matrix_4fv<T: Copy>(
        &self,
        location: &LocationId,
        data: &T,
        count: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.UniformMatrix4fv(
                location.0 as _,
                count as _,
                gles3_bindings::FALSE,
                data as *const T as _,
            );
            self.check_for_error()
        }
    }

    pub fn gl_active_texture(
        &self,
        i: u32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.ActiveTexture(gles3_bindings::TEXTURE0 + i);
            self.check_for_error()
        }
    }

    pub fn gl_pixel_storei(
        &self,
        pname: GLenum,
        param: i32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.PixelStorei(pname, param);
            self.check_for_error()
        }
    }

    pub fn gl_bind_texture(
        &self,
        target: GLenum,
        texture_id: TextureId,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.BindTexture(target, texture_id.0);
            self.check_for_error()
        }
    }

    pub fn gl_tex_image_2d(
        &self,
        target: GLenum,
        mip_level: u8,
        internal_format: i32,
        width: u32,
        height: u32,
        border: i32,
        format: GLenum,
        type_: u32,
        pixels: Option<&[u8]>,
    ) -> RafxResult<()> {
        unsafe {
            let pixels_ptr = pixels.map(|x| x.as_ptr()).unwrap_or(std::ptr::null());
            self.gles3.TexImage2D(
                target,
                mip_level as _,
                internal_format,
                width as _,
                height as _,
                border,
                format,
                type_,
                pixels_ptr as _,
            );
            self.check_for_error()
        }
    }

    pub fn gl_tex_parameteri(
        &self,
        target: GLenum,
        pname: GLenum,
        param: i32,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.TexParameteri(target, pname, param);
            self.check_for_error()
        }
    }

    pub fn gl_generate_mipmap(
        &self,
        target: GLenum,
    ) -> RafxResult<()> {
        unsafe {
            self.gles3.GenerateMipmap(target);
            self.check_for_error()
        }
    }
}

fn to_gl_bool(value: bool) -> GLboolean {
    if value {
        gles3_bindings::TRUE
    } else {
        gles3_bindings::FALSE
    }
}

pub fn check_for_error(gles3: &Gles3) -> RafxResult<()> {
    unsafe {
        let result = gles3.GetError();
        if result != gles3_bindings::NO_ERROR {
            Err(RafxError::GlError(result))
        } else {
            Ok(())
        }
    }
}

pub fn gl_get_integerv(
    gles3: &Gles3,
    pname: u32,
) -> i32 {
    unsafe {
        let mut value = 0;
        gles3.GetIntegerv(pname, &mut value);
        value
    }
}

pub fn gl_get_string(
    gles3: &Gles3,
    pname: u32,
) -> String {
    unsafe {
        let str = gles3.GetString(pname);
        if str.is_null() {
            return "".to_string();
        }

        std::ffi::CStr::from_ptr(str as _)
            .to_str()
            .unwrap()
            .to_string()
    }
}

// This is not public because it is not supported in ES 2.0. However we need it internally so it's
// defined here.
fn gl_get_stringi(
    gles3: &Gles3,
    pname: u32,
    index: u32,
) -> String {
    if !gles3.GetStringi.is_loaded() {
        panic!("This function is not supported in base GL ES 2.0. Must verify that it is loaded before calling it!");
    }

    unsafe {
        let str = gles3.GetStringi(pname, index);
        if str.is_null() {
            return "".to_string();
        }

        std::ffi::CStr::from_ptr(str as _)
            .to_str()
            .unwrap()
            .to_string()
    }
}
