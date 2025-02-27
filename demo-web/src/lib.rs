#[cfg(not(target_arch = "wasm32"))]
mod main_native;
#[cfg(not(target_arch = "wasm32"))]
pub use main_native::*;

#[cfg(target_arch = "wasm32")]
mod main_web;
#[cfg(target_arch = "wasm32")]
pub use main_web::*;

use rafx::api::*;
use winit::{
    event::{Event, WindowEvent},
    event_loop::ControlFlow,
};

pub fn update_loop(
    window: winit::window::Window,
    event_loop: winit::event_loop::EventLoop<()>,
) -> RafxResult<()> {
    //
    // Create the api
    //
    log::trace!("Creating the API");
    let api = unsafe { RafxApi::new(&window, &window, &Default::default())? };

    // Wrap all of this so that it gets dropped before we drop the API object. This ensures a nice
    // clean shutdown.
    {
        // A cloneable device handle, these are lightweight and can be passed across threads
        let device_context = api.device_context();

        //
        // Allocate a graphics queue. By default, there is just one graphics queue and it is shared.
        // There currently is no API for customizing this but the code would be easy to adapt to act
        // differently. Most recommendations I've seen are to just use one graphics queue. (The
        // rendering hardware is shared among them)
        //
        log::trace!("Creating graphics queue");
        let graphics_queue = device_context.create_queue(RafxQueueType::Graphics)?;

        //
        // Create a swapchain
        //
        log::trace!("Creating swapchain");
        let window_size = window.inner_size();
        let swapchain = device_context.create_swapchain(
            &window,
            &window,
            &graphics_queue,
            &RafxSwapchainDef {
                width: window_size.width,
                height: window_size.height,
                enable_vsync: true,
                color_space_priority: vec![RafxSwapchainColorSpace::Srgb],
            },
        )?;

        //
        // Wrap the swapchain in this helper to cut down on boilerplate. This helper is
        // multithreaded-rendering friendly! The PresentableFrame it returns can be sent to another
        // thread and presented from there, and any errors are returned back to the main thread
        // when the next image is acquired. The helper also ensures that the swapchain is rebuilt
        // as necessary.
        //
        log::trace!("Creating swapchain helper");
        let mut swapchain_helper = RafxSwapchainHelper::new(&device_context, swapchain, None)?;

        //
        // Some default data we can render
        //
        #[rustfmt::skip]
        let vertex_data = [
            -0.5f32, -0.5, 0.0, 1.0,
            -0.5, 0.5, 0.0, 0.0,
            0.5, 0.5, 1.0, 0.0,
            -0.5, -0.5, 0.0, 1.0,
            0.5, 0.5, 1.0, 0.0,
            0.5, -0.5, 1.0, 1.0
        ];

        let uniform_data = [1.0f32, 0.0, 1.0, 1.0];

        //
        // Create command pools/command buffers. The command pools need to be immutable while they are
        // being processed by a queue, so create one per swapchain image.
        //
        // Create vertex buffers (with position/color information) and a uniform buffers that we
        // can bind to pass additional info.
        //
        // In this demo, the color data in the shader is pulled from
        // the uniform instead of the vertex buffer. Buffers also need to be immutable while
        // processed, so we need one per swapchain image
        //
        let mut command_pools = Vec::with_capacity(swapchain_helper.image_count());
        let mut command_buffers = Vec::with_capacity(swapchain_helper.image_count());
        let mut vertex_buffers = Vec::with_capacity(swapchain_helper.image_count());
        let mut uniform_buffers = Vec::with_capacity(swapchain_helper.image_count());

        for _ in 0..swapchain_helper.image_count() {
            log::trace!("Creating command pool");
            let mut command_pool =
                graphics_queue.create_command_pool(&RafxCommandPoolDef { transient: true })?;

            log::trace!("Creating command buffer");
            let command_buffer = command_pool.create_command_buffer(&RafxCommandBufferDef {
                is_secondary: false,
            })?;

            log::trace!("Creating vertex buffer");
            let vertex_buffer = device_context
                .create_buffer(&RafxBufferDef::for_staging_vertex_buffer_data(&vertex_data))?;
            log::trace!("Populating vertex buffer");
            vertex_buffer.copy_to_host_visible_buffer(&vertex_data)?;

            log::trace!("Creating uniform buffer");
            let uniform_buffer = device_context.create_buffer(
                &RafxBufferDef::for_staging_uniform_buffer_data(&uniform_data),
            )?;
            uniform_buffer.copy_to_host_visible_buffer(&uniform_data)?;

            command_pools.push(command_pool);
            command_buffers.push(command_buffer);
            vertex_buffers.push(vertex_buffer);
            uniform_buffers.push(uniform_buffer);
        }

        use image::GenericImageView;
        let nyancat_image = image::load_from_memory_with_format(
            include_bytes!("../nyancat.jpg"),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
        let (image_width, image_height) = nyancat_image.dimensions();
        let image_data = nyancat_image.to_rgba8().into_raw();
        let texture = device_context.create_texture(&RafxTextureDef {
            extents: RafxExtents3D {
                width: image_width,
                height: image_height,
                depth: 1,
            },
            format: RafxFormat::R8G8B8A8_UNORM,
            ..Default::default()
        })?;

        let texture_staging_buffer = device_context
            .create_buffer(&RafxBufferDef::for_staging_buffer_data(
                image_data.as_slice(),
                RafxResourceType::TEXTURE,
            ))
            .unwrap();
        texture_staging_buffer
            .copy_to_host_visible_buffer(image_data.as_slice())
            .unwrap();

        let mut upload_command_pool =
            graphics_queue.create_command_pool(&RafxCommandPoolDef { transient: true })?;

        let upload_command_buffer = upload_command_pool
            .create_command_buffer(&RafxCommandBufferDef {
                is_secondary: false,
            })
            .unwrap();

        upload_command_buffer.begin().unwrap();
        upload_command_buffer
            .cmd_copy_buffer_to_texture(
                &texture_staging_buffer,
                &texture,
                &RafxCmdCopyBufferToTextureParams {
                    mip_level: 0,
                    array_layer: 0,
                    buffer_offset: 0,
                },
            )
            .unwrap();
        upload_command_buffer.end().unwrap();
        graphics_queue
            .submit(&[&upload_command_buffer], &[], &[], None)
            .unwrap();
        graphics_queue.wait_for_queue_idle().unwrap();

        let sampler = device_context
            .create_sampler(&RafxSamplerDef::default())
            .unwrap();

        //
        // Load a shader from source - this part is API-specific. vulkan will want SPV, metal wants
        // source code or even better a pre-compiled library. The web demo is GL-only, and it only
        // supports loading from src.
        //
        // The resulting shader modules represent a loaded shader GPU object that is used to create
        // shaders. Shader modules can be discarded once the graphics pipeline is built.
        //
        log::trace!("Creating shader modules");
        let vert_shader_module = device_context.create_shader_module(RafxShaderModuleDef {
            gles2: Some(RafxShaderModuleDefGles2::GlSrc(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/processed_shaders/shader.vert.gles2"
            )))),
            ..Default::default()
        })?;

        let frag_shader_module = device_context.create_shader_module(RafxShaderModuleDef {
            gles2: Some(RafxShaderModuleDefGles2::GlSrc(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/processed_shaders/shader.frag.gles2"
            )))),
            ..Default::default()
        })?;

        //
        // Create the shader object by combining the stages
        //
        // Hardcode the reflecton data required to interact with the shaders. This can be generated
        // offline and loaded with the shader but this is not currently provided in rafx-api itself.
        // (But see the shader pipeline in higher-level rafx crates for example usage, generated
        // from spirv_cross)
        //
        log::trace!("Creating shader resources");
        let uniform_resource = RafxShaderResource {
            name: Some("uniform_data".to_string()),
            set_index: 0,
            binding: 0,
            resource_type: RafxResourceType::UNIFORM_BUFFER,
            gles_name: Some("uniform_data".to_string()),
            gles2_uniform_members: vec![RafxGlUniformMember::new("uniform_data.mvp", 0)],
            ..Default::default()
        };

        let texture_resource = RafxShaderResource {
            name: Some("tex".to_string()),
            set_index: 0,
            binding: 1,
            resource_type: RafxResourceType::TEXTURE,
            gles_name: Some("tex".to_string()),
            gles_sampler_name: Some("smp".to_string()),
            ..Default::default()
        };

        let sampler_resource = RafxShaderResource {
            name: Some("smp".to_string()),
            set_index: 0,
            binding: 2,
            resource_type: RafxResourceType::SAMPLER,
            gles_name: Some("smp".to_string()),
            ..Default::default()
        };

        let vert_shader_stage_def = RafxShaderStageDef {
            shader_module: vert_shader_module,
            reflection: RafxShaderStageReflection {
                entry_point_name: "main".to_string(),
                shader_stage: RafxShaderStageFlags::VERTEX,
                compute_threads_per_group: None,
                resources: vec![uniform_resource],
            },
        };

        let frag_shader_stage_def = RafxShaderStageDef {
            shader_module: frag_shader_module,
            reflection: RafxShaderStageReflection {
                entry_point_name: "main".to_string(),
                shader_stage: RafxShaderStageFlags::FRAGMENT,
                compute_threads_per_group: None,
                resources: vec![sampler_resource, texture_resource],
            },
        };

        //
        // Combine the shader stages into a single shader
        //
        log::trace!("Creating shader");
        let shader =
            device_context.create_shader(vec![vert_shader_stage_def, frag_shader_stage_def])?;

        //
        // Create the root signature object - it represents the pipeline layout and can be shared among
        // shaders. But one per shader is fine.
        //
        log::trace!("Creating root signature");
        let root_signature = device_context.create_root_signature(&RafxRootSignatureDef {
            shaders: &[shader.clone()],
            immutable_samplers: &[RafxImmutableSamplers {
                key: RafxImmutableSamplerKey::Name("smp"),
                samplers: &[sampler],
            }],
        })?;

        //
        // Descriptors are allocated in blocks and never freed. Normally you will want to build a
        // pooling system around this. (Higher-level rafx crates provide this.) But they're small
        // and cheap. We need one per swapchain image.
        //
        log::trace!("Creating descriptor set array");
        let mut descriptor_set_array =
            device_context.create_descriptor_set_array(&RafxDescriptorSetArrayDef {
                set_index: 0,
                root_signature: &root_signature,
                array_length: 3, // One per swapchain image.
            })?;

        // Initialize them all at once here.. this can be done per-frame as well.
        log::trace!("Set up descriptor sets");
        for i in 0..swapchain_helper.image_count() {
            descriptor_set_array.update_descriptor_set(&[
                RafxDescriptorUpdate {
                    array_index: i as u32,
                    descriptor_key: RafxDescriptorKey::Name("uniform_data"),
                    elements: RafxDescriptorElements {
                        buffers: Some(&[&uniform_buffers[i]]),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                RafxDescriptorUpdate {
                    array_index: i as u32,
                    descriptor_key: RafxDescriptorKey::Name("tex"),
                    elements: RafxDescriptorElements {
                        textures: Some(&[&texture]),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ])?;
        }

        //
        // Now set up the pipeline. LOTS of things can be configured here, but aside from the vertex
        // layout most of it can be left as default.
        //
        let vertex_layout = RafxVertexLayout {
            attributes: vec![
                RafxVertexLayoutAttribute {
                    format: RafxFormat::R32G32_SFLOAT,
                    buffer_index: 0,
                    location: 0,
                    byte_offset: 0,
                    hlsl_semantic: "POSITION".to_string(),
                    gl_attribute_name: Some("pos".to_string()),
                },
                RafxVertexLayoutAttribute {
                    format: RafxFormat::R32G32_SFLOAT,
                    buffer_index: 0,
                    location: 1,
                    byte_offset: 8,
                    hlsl_semantic: "TEXCOORD".to_string(),
                    gl_attribute_name: Some("in_uv".to_string()),
                },
            ],
            buffers: vec![RafxVertexLayoutBuffer {
                stride: 16,
                rate: RafxVertexAttributeRate::Vertex,
            }],
        };

        let pipeline = device_context.create_graphics_pipeline(&RafxGraphicsPipelineDef {
            shader: &shader,
            root_signature: &root_signature,
            vertex_layout: &vertex_layout,
            blend_state: &Default::default(),
            depth_state: &Default::default(),
            rasterizer_state: &Default::default(),
            color_formats: &[swapchain_helper.format()],
            sample_count: RafxSampleCount::SampleCount1,
            depth_stencil_format: None,
            primitive_topology: RafxPrimitiveTopology::TriangleList,
            debug_name: None,
        })?;

        log::trace!("Starting event loop");
        let start_time = rafx::base::Instant::now();

        event_loop.run(move |event, _, control_flow| {
            *control_flow = ControlFlow::Poll;

            match event {
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => *control_flow = ControlFlow::Exit,
                Event::MainEventsCleared => {
                    window.request_redraw();
                }
                Event::WindowEvent {
                    event: window_event,
                    ..
                } => match window_event {
                    WindowEvent::KeyboardInput { .. } | WindowEvent::MouseInput { .. } => {
                        log::debug!("{:?}", window_event);
                    }
                    _ => {}
                },
                Event::RedrawRequested(_) => {
                    let elapsed_seconds = start_time.elapsed().as_secs_f32();

                    let m = elapsed_seconds.cos();

                    #[rustfmt::skip]
                    let vertex_data = [
                        m * -0.5, -0.5, 0.0, 1.0,
                        m * -0.5, 0.5, 0.0, 0.0,
                        m * 0.5, 0.5, 1.0, 0.0,
                        m * -0.5, -0.5, 0.0, 1.0,
                        m * 0.5, 0.5, 1.0, 0.0,
                        m * 0.5, -0.5, 1.0, 1.0,
                    ];

                    //
                    // Acquire swapchain image
                    //
                    let window_size = window.inner_size();
                    let presentable_frame = swapchain_helper
                        .acquire_next_image(window_size.width, window_size.height, None)
                        .unwrap();
                    let swapchain_texture = presentable_frame.swapchain_texture();

                    //
                    // Use the command pool/buffer assigned to this frame
                    //
                    let cmd_pool = &mut command_pools[presentable_frame.rotating_frame_index()];
                    let cmd_buffer = &command_buffers[presentable_frame.rotating_frame_index()];
                    let vertex_buffer = &vertex_buffers[presentable_frame.rotating_frame_index()];
                    let uniform_buffer = &uniform_buffers[presentable_frame.rotating_frame_index()];

                    //
                    // Update the buffers
                    //
                    vertex_buffer
                        .copy_to_host_visible_buffer(&vertex_data)
                        .unwrap();
                    uniform_buffer
                        .copy_to_host_visible_buffer(&uniform_data)
                        .unwrap();

                    //
                    // Record the command buffer. For now just transition it between layouts
                    //
                    cmd_pool.reset_command_pool().unwrap();
                    cmd_buffer.begin().unwrap();

                    // Put it into a layout where we can draw on it
                    cmd_buffer
                        .cmd_resource_barrier(
                            &[],
                            &[RafxTextureBarrier::state_transition(
                                &swapchain_texture,
                                RafxResourceState::PRESENT,
                                RafxResourceState::RENDER_TARGET,
                            )],
                        )
                        .unwrap();

                    cmd_buffer
                        .cmd_begin_render_pass(
                            &[RafxColorRenderTargetBinding {
                                texture: &swapchain_texture,
                                load_op: RafxLoadOp::Clear,
                                store_op: RafxStoreOp::Store,
                                array_slice: None,
                                mip_slice: None,
                                clear_value: RafxColorClearValue([0.2, 0.2, 0.2, 1.0]),
                                resolve_target: None,
                                resolve_store_op: RafxStoreOp::DontCare,
                                resolve_mip_slice: None,
                                resolve_array_slice: None,
                            }],
                            None,
                        )
                        .unwrap();

                    cmd_buffer.cmd_bind_pipeline(&pipeline).unwrap();

                    cmd_buffer
                        .cmd_bind_vertex_buffers(
                            0,
                            &[RafxVertexBufferBinding {
                                buffer: &vertex_buffer,
                                byte_offset: 0,
                            }],
                        )
                        .unwrap();

                    cmd_buffer
                        .cmd_bind_descriptor_set(
                            &descriptor_set_array,
                            presentable_frame.rotating_frame_index() as u32,
                        )
                        .unwrap();
                    cmd_buffer.cmd_draw(6, 0).unwrap();

                    cmd_buffer.cmd_end_render_pass().unwrap();

                    // Put it into a layout where we can present it
                    cmd_buffer
                        .cmd_resource_barrier(
                            &[],
                            &[RafxTextureBarrier::state_transition(
                                &swapchain_texture,
                                RafxResourceState::RENDER_TARGET,
                                RafxResourceState::PRESENT,
                            )],
                        )
                        .unwrap();

                    cmd_buffer.end().unwrap();

                    //
                    // Present the image
                    //
                    presentable_frame
                        .present(&graphics_queue, &[&cmd_buffer])
                        .unwrap();
                }
                _ => (),
            }
        });
    }
}

pub fn update() {}
