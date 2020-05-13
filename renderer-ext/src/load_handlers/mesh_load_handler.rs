use renderer_shell_vulkan::{
    VkTransferUploadState, VkDevice, VkDeviceContext, VkTransferUpload, VkImage, VkBuffer,
};
use crossbeam_channel::{Sender, Receiver};
use ash::prelude::VkResult;
use std::time::Duration;
use crate::image_utils::{enqueue_load_images, DecodedTexture};
use std::mem::{ManuallyDrop, align_of};
use crate::asset_storage::{ResourceHandle, ResourceLoadHandler};
use std::error::Error;
use atelier_assets::core::AssetUuid;
use atelier_assets::loader::{LoadHandle, AssetLoadOp};
use fnv::FnvHashMap;
use std::sync::Arc;
use image::load;

use crate::upload::{PendingImageUpload, PendingBufferUpload};
use crate::upload::BufferUploadOpResult;
use crate::upload::BufferUploadOpAwaiter;
use crate::resource_managers::mesh_resource_manager::{
    MeshUpdate, LoadingMeshPartRenderInfo, LoadingMeshRenderInfo,
};
use crate::pipeline::gltf::MeshAsset;
use crate::push_buffer::{PushBufferSizeCalculator, PushBuffer};

struct PendingMeshUpdate {
    awaiter: BufferUploadOpAwaiter,
    mesh_parts: Vec<LoadingMeshPartRenderInfo>,
}

// This is registered with the asset storage which lets us hook when assets are updated
pub struct MeshLoadHandler {
    upload_tx: Sender<PendingBufferUpload>,
    mesh_update_tx: Sender<MeshUpdate>,
    pending_updates: FnvHashMap<LoadHandle, FnvHashMap<u32, PendingMeshUpdate>>,
}

impl MeshLoadHandler {
    pub fn new(
        upload_tx: Sender<PendingBufferUpload>,
        mesh_update_tx: Sender<MeshUpdate>,
    ) -> Self {
        MeshLoadHandler {
            upload_tx,
            mesh_update_tx,
            pending_updates: Default::default(),
        }
    }
}

// This sends the texture to the upload queue. The upload queue will batch uploads together when update()
// is called on it. When complete, the upload queue will send the image handle back via a channel
impl ResourceLoadHandler<MeshAsset> for MeshLoadHandler {
    fn update_asset(
        &mut self,
        load_handle: LoadHandle,
        asset_uuid: &AssetUuid,
        resource_handle: ResourceHandle<MeshAsset>,
        version: u32,
        asset: &MeshAsset,
        load_op: AssetLoadOp,
    ) {
        log::info!(
            "MeshLoadHandler update_asset {} {:?} {:?}",
            version,
            load_handle,
            resource_handle
        );
        let (upload_op, awaiter) = crate::upload::create_upload_op();

        //
        // Determine size of buffer needed
        //
        // Arbitrary, not sure if there is any requirement
        const REQUIRED_ALIGNMENT: usize = 16;
        let mut storage_calculator = PushBufferSizeCalculator::new();
        for mesh_part in &asset.mesh_parts {
            storage_calculator.push(&mesh_part.indices, REQUIRED_ALIGNMENT);
            storage_calculator.push(&mesh_part.vertices, REQUIRED_ALIGNMENT);
        }

        //
        // Concatenate vertex/index data for all mesh parts into a buffer
        //
        let mut mesh_part_render_infos = Vec::with_capacity(asset.mesh_parts.len());
        let mut combined_mesh_data = PushBuffer::new(storage_calculator.required_size());
        for mesh_part in &asset.mesh_parts {
            let index = combined_mesh_data.push(&mesh_part.indices, REQUIRED_ALIGNMENT);
            let vertex = combined_mesh_data.push(&mesh_part.vertices, REQUIRED_ALIGNMENT);

            mesh_part_render_infos.push(LoadingMeshPartRenderInfo {
                index_offset: index.offset() as u32,
                index_size: index.size() as u32,
                vertex_offset: vertex.offset() as u32,
                vertex_size: vertex.size() as u32,
                material: mesh_part.material,
            });
        }

        let pending_update = PendingMeshUpdate {
            awaiter,
            mesh_parts: mesh_part_render_infos,
        };

        self.pending_updates
            .entry(load_handle)
            .or_default()
            .insert(version, pending_update);

        self.upload_tx
            .send(PendingBufferUpload {
                load_op,
                upload_op,
                data: combined_mesh_data.into_data(),
            })
            .unwrap(); //TODO: Better error handling
    }

    fn commit_asset_version(
        &mut self,
        load_handle: LoadHandle,
        asset_uuid: &AssetUuid,
        resource_handle: ResourceHandle<MeshAsset>,
        version: u32,
        asset: &MeshAsset,
    ) {
        log::info!(
            "MeshLoadHandler commit_asset_version {} {:?} {:?}",
            version,
            load_handle,
            resource_handle
        );
        if let Some(versions) = self.pending_updates.get_mut(&load_handle) {
            if let Some(pending_update) = versions.remove(&version) {
                let awaiter = pending_update.awaiter;

                // We assume that if commit_asset_version is being called the awaiter is signaled
                // and has a valid result
                let value = awaiter
                    .receiver()
                    .recv_timeout(Duration::from_secs(0))
                    .unwrap();
                match value {
                    BufferUploadOpResult::UploadComplete(buffer) => {
                        log::info!("Commit asset {:?} {:?}", load_handle, version);

                        let mesh_render_info = LoadingMeshRenderInfo {
                            buffer,
                            mesh_parts: pending_update.mesh_parts,
                        };

                        self.mesh_update_tx.send(MeshUpdate {
                            meshes: vec![mesh_render_info],
                            resource_handles: vec![resource_handle],
                        });
                    }
                    BufferUploadOpResult::UploadError => unreachable!(),
                    BufferUploadOpResult::UploadDrop => unreachable!(),
                }
            } else {
                log::error!(
                    "Could not find awaiter for asset version {:?} {}",
                    load_handle,
                    version
                );
            }
        } else {
            log::error!("Could not find awaiter for {:?} {}", load_handle, version);
        }
    }

    fn free(
        &mut self,
        load_handle: LoadHandle,
        resource_handle: ResourceHandle<MeshAsset>,
    ) {
        log::info!(
            "MeshLoadHandler free {:?} {:?}",
            load_handle,
            resource_handle
        );

        //TODO: We are not unloading meshes

        self.pending_updates.remove(&load_handle);
    }
}
