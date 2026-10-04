//! Vulkan layer plumbing: the loader calls our `GetInstanceProcAddr` /
//! `GetDeviceProcAddr`; we wrap instance and device creation to learn the
//! next layer's entry points, swapchain creation to know the images, and
//! `vkQueuePresentKHR` to draw the overlay before each present.

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::sync::{Arc, Mutex};

use ash::vk::{self, Handle};

use crate::linux::client::Client;
use crate::linux::render::{Gpu, Renderer, SetDeviceLoaderData};

// From vk_layer.h, which ash does not cover.
const LOADER_INSTANCE_CREATE_INFO: vk::StructureType = vk::StructureType::from_raw(47);
const LOADER_DEVICE_CREATE_INFO: vk::StructureType = vk::StructureType::from_raw(48);
const LAYER_LINK_INFO: i32 = 0;
const LOADER_DATA_CALLBACK: i32 = 1;

#[repr(C)]
struct LayerInstanceLink {
    next: *mut LayerInstanceLink,
    next_get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    next_get_physical_device_proc_addr: *const c_void,
}

#[repr(C)]
struct LayerDeviceLink {
    next: *mut LayerDeviceLink,
    next_get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    next_get_device_proc_addr: vk::PFN_vkGetDeviceProcAddr,
}

/// VkLayerInstanceCreateInfo and VkLayerDeviceCreateInfo: `u` is a union of
/// a link pointer and loader callbacks.
#[repr(C)]
struct LayerCreateInfo {
    s_type: vk::StructureType,
    p_next: *const c_void,
    function: i32,
    u: *mut c_void,
}

struct InstanceData {
    instance: ash::Instance,
    next_gipa: vk::PFN_vkGetInstanceProcAddr,
}

struct DeviceData {
    gpu: Gpu,
    next_gdpa: vk::PFN_vkGetDeviceProcAddr,
    swapchain_fn: ash::khr::swapchain::DeviceFn,
    /// Queue handle -> (family, family supports graphics).
    queues: HashMap<u64, (u32, bool)>,
    renderer: Option<Renderer>,
    client: Client,
}

// Raw handles only; access goes through the mutexes below.
unsafe impl Send for DeviceData {}
unsafe impl Sync for InstanceData {}
unsafe impl Send for InstanceData {}

static INSTANCES: Mutex<Option<HashMap<usize, Arc<InstanceData>>>> = Mutex::new(None);
static DEVICES: Mutex<Option<HashMap<usize, Arc<Mutex<DeviceData>>>>> = Mutex::new(None);

/// The loader's dispatch table pointer, shared by an object and its children.
unsafe fn key<T: Handle>(handle: T) -> usize {
    *(handle.as_raw() as *const usize)
}

fn instance_data(k: usize) -> Option<Arc<InstanceData>> {
    INSTANCES.lock().unwrap().as_ref()?.get(&k).cloned()
}

fn device_data(k: usize) -> Option<Arc<Mutex<DeviceData>>> {
    DEVICES.lock().unwrap().as_ref()?.get(&k).cloned()
}

/// Walks the create info chain to our layer's entry of type `s_type` / `function`.
unsafe fn find_link(mut next: *const c_void, s_type: vk::StructureType, function: i32) -> *mut LayerCreateInfo {
    while !next.is_null() {
        let info = next as *mut LayerCreateInfo;
        if (*info).s_type == s_type && (*info).function == function {
            return info;
        }
        next = (*info).p_next;
    }
    std::ptr::null_mut()
}

macro_rules! fp {
    ($f:expr) => {
        std::mem::transmute::<*const (), unsafe extern "system" fn()>($f as *const ())
    };
}

#[no_mangle]
pub unsafe extern "system" fn gameviber_GetInstanceProcAddr(instance: vk::Instance, name: *const c_char) -> vk::PFN_vkVoidFunction {
    if let Some(f) = intercept(CStr::from_ptr(name)) {
        return Some(f);
    }
    if instance == vk::Instance::null() {
        return None;
    }
    let data = instance_data(key(instance))?;
    (data.next_gipa)(instance, name)
}

#[no_mangle]
pub unsafe extern "system" fn gameviber_GetDeviceProcAddr(device: vk::Device, name: *const c_char) -> vk::PFN_vkVoidFunction {
    if let Some(f) = intercept_device(CStr::from_ptr(name)) {
        return Some(f);
    }
    let data = device_data(key(device))?;
    let gdpa = data.lock().unwrap().next_gdpa;
    gdpa(device, name)
}

unsafe fn intercept(name: &CStr) -> Option<unsafe extern "system" fn()> {
    Some(match name.to_bytes() {
        b"vkGetInstanceProcAddr" => fp!(gameviber_GetInstanceProcAddr as vk::PFN_vkGetInstanceProcAddr),
        b"vkCreateInstance" => fp!(create_instance as vk::PFN_vkCreateInstance),
        b"vkDestroyInstance" => fp!(destroy_instance as vk::PFN_vkDestroyInstance),
        b"vkCreateDevice" => fp!(create_device as vk::PFN_vkCreateDevice),
        _ => return intercept_device(name),
    })
}

unsafe fn intercept_device(name: &CStr) -> Option<unsafe extern "system" fn()> {
    Some(match name.to_bytes() {
        b"vkGetDeviceProcAddr" => fp!(gameviber_GetDeviceProcAddr as vk::PFN_vkGetDeviceProcAddr),
        b"vkDestroyDevice" => fp!(destroy_device as vk::PFN_vkDestroyDevice),
        b"vkCreateSwapchainKHR" => fp!(create_swapchain as vk::PFN_vkCreateSwapchainKHR),
        b"vkDestroySwapchainKHR" => fp!(destroy_swapchain as vk::PFN_vkDestroySwapchainKHR),
        b"vkQueuePresentKHR" => fp!(queue_present as vk::PFN_vkQueuePresentKHR),
        _ => return None,
    })
}

unsafe extern "system" fn create_instance(
    info: *const vk::InstanceCreateInfo,
    alloc: *const vk::AllocationCallbacks,
    out: *mut vk::Instance,
) -> vk::Result {
    let link = find_link((*info).p_next, LOADER_INSTANCE_CREATE_INFO, LAYER_LINK_INFO);
    if link.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    let layer = (*link).u as *mut LayerInstanceLink;
    let next_gipa = (*layer).next_get_instance_proc_addr;
    // Hand the rest of the chain to the next layer.
    (*link).u = (*layer).next as *mut c_void;
    let Some(create) = next_gipa(vk::Instance::null(), c"vkCreateInstance".as_ptr()) else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    let create: vk::PFN_vkCreateInstance = std::mem::transmute(create);
    let result = create(info, alloc, out);
    if result != vk::Result::SUCCESS {
        return result;
    }
    let instance = ash::Instance::load(&ash::StaticFn { get_instance_proc_addr: next_gipa }, *out);
    INSTANCES.lock().unwrap().get_or_insert_with(HashMap::new).insert(key(*out), Arc::new(InstanceData { instance, next_gipa }));
    vk::Result::SUCCESS
}

unsafe extern "system" fn destroy_instance(instance: vk::Instance, alloc: *const vk::AllocationCallbacks) {
    let data = INSTANCES.lock().unwrap().as_mut().and_then(|m| m.remove(&key(instance)));
    if let Some(data) = data {
        (data.instance.fp_v1_0().destroy_instance)(instance, alloc);
    }
}

unsafe extern "system" fn create_device(
    physical: vk::PhysicalDevice,
    info: *const vk::DeviceCreateInfo,
    alloc: *const vk::AllocationCallbacks,
    out: *mut vk::Device,
) -> vk::Result {
    let Some(inst) = instance_data(key(physical)) else { return vk::Result::ERROR_INITIALIZATION_FAILED };
    let link = find_link((*info).p_next, LOADER_DEVICE_CREATE_INFO, LAYER_LINK_INFO);
    let callback = find_link((*info).p_next, LOADER_DEVICE_CREATE_INFO, LOADER_DATA_CALLBACK);
    if link.is_null() || callback.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    let layer = (*link).u as *mut LayerDeviceLink;
    let next_gipa = (*layer).next_get_instance_proc_addr;
    let next_gdpa = (*layer).next_get_device_proc_addr;
    (*link).u = (*layer).next as *mut c_void;
    let set_loader_data: SetDeviceLoaderData = std::mem::transmute((*callback).u);
    let Some(create) = next_gipa(inst.instance.handle(), c"vkCreateDevice".as_ptr()) else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    let create: vk::PFN_vkCreateDevice = std::mem::transmute(create);
    let result = create(physical, info, alloc, out);
    if result != vk::Result::SUCCESS {
        return result;
    }
    let device = *out;

    let mut instance_fn = inst.instance.fp_v1_0().clone();
    instance_fn.get_device_proc_addr = next_gdpa;
    let ash_device = ash::Device::load(&instance_fn, device);
    let swapchain_fn = ash::khr::swapchain::DeviceFn::load(|name| {
        std::mem::transmute::<vk::PFN_vkVoidFunction, *const c_void>(next_gdpa(device, name.as_ptr()))
    });
    let families = inst.instance.get_physical_device_queue_family_properties(physical);
    let mut queues = HashMap::new();
    let queue_infos = std::slice::from_raw_parts((*info).p_queue_create_infos, (*info).queue_create_info_count as usize);
    for q in queue_infos.iter().filter(|q| q.flags.is_empty()) {
        let graphics = families.get(q.queue_family_index as usize).is_some_and(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS));
        for i in 0..q.queue_count {
            // Queues are identified by handle: the loader has not set their dispatch yet.
            let queue = ash_device.get_device_queue(q.queue_family_index, i);
            queues.insert(queue.as_raw(), (q.queue_family_index, graphics));
        }
    }
    let gpu = Gpu {
        device: ash_device,
        instance: inst.instance.clone(),
        physical,
        memory: inst.instance.get_physical_device_memory_properties(physical),
        set_loader_data,
    };
    let data = DeviceData { gpu, next_gdpa, swapchain_fn, queues, renderer: None, client: Client::new("vulkan") };
    DEVICES.lock().unwrap().get_or_insert_with(HashMap::new).insert(key(device), Arc::new(Mutex::new(data)));
    vk::Result::SUCCESS
}

unsafe extern "system" fn destroy_device(device: vk::Device, alloc: *const vk::AllocationCallbacks) {
    let data = DEVICES.lock().unwrap().as_mut().and_then(|m| m.remove(&key(device)));
    let Some(data) = data else { return };
    let mut data = data.lock().unwrap();
    if let Some(renderer) = data.renderer.take() {
        let _ = data.gpu.device.device_wait_idle();
        renderer.destroy(&data.gpu);
    }
    (data.gpu.device.fp_v1_0().destroy_device)(device, alloc);
}

unsafe extern "system" fn create_swapchain(
    device: vk::Device,
    info: *const vk::SwapchainCreateInfoKHR,
    alloc: *const vk::AllocationCallbacks,
    out: *mut vk::SwapchainKHR,
) -> vk::Result {
    let Some(data) = device_data(key(device)) else { return vk::Result::ERROR_INITIALIZATION_FAILED };
    let mut data = data.lock().unwrap();
    // The overlay renders into the images, and copies them for GameViber.
    let requested = (*info).image_usage;
    let mut info = *info;
    info.image_usage |= vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC;
    let mut result = (data.swapchain_fn.create_swapchain_khr)(device, &info, alloc, out);
    if result != vk::Result::SUCCESS && !requested.contains(vk::ImageUsageFlags::TRANSFER_SRC) {
        // Some surfaces cannot be copied from: draw without copying.
        info.image_usage &= !vk::ImageUsageFlags::TRANSFER_SRC;
        result = (data.swapchain_fn.create_swapchain_khr)(device, &info, alloc, out);
    }
    if result != vk::Result::SUCCESS {
        return result;
    }
    if let Err(e) = data.add_swapchain(*out, &info).result() {
        crate::log(&format!("overlay disabled for this swapchain: {e}"));
    }
    vk::Result::SUCCESS
}

unsafe extern "system" fn destroy_swapchain(device: vk::Device, swapchain: vk::SwapchainKHR, alloc: *const vk::AllocationCallbacks) {
    let Some(data) = device_data(key(device)) else { return };
    let mut data = data.lock().unwrap();
    let DeviceData { gpu, renderer, .. } = &mut *data;
    if let Some(r) = renderer.as_mut() {
        r.remove_swapchain(gpu, swapchain);
    }
    (data.swapchain_fn.destroy_swapchain_khr)(device, swapchain, alloc);
}

impl DeviceData {
    unsafe fn add_swapchain(&mut self, swapchain: vk::SwapchainKHR, info: &vk::SwapchainCreateInfoKHR) -> vk::Result {
        let device = self.gpu.device.handle();
        let mut count = 0;
        let get = self.swapchain_fn.get_swapchain_images_khr;
        let r = get(device, swapchain, &mut count, std::ptr::null_mut());
        if r != vk::Result::SUCCESS {
            return r;
        }
        let mut images = vec![vk::Image::null(); count as usize];
        let r = get(device, swapchain, &mut count, images.as_mut_ptr());
        if r != vk::Result::SUCCESS {
            return r;
        }
        if self.renderer.is_none() {
            match Renderer::new(&self.gpu) {
                Ok(r) => self.renderer = Some(r),
                Err(e) => return e,
            }
        }
        let renderer = self.renderer.as_mut().unwrap();
        let transfer = info.image_usage.contains(vk::ImageUsageFlags::TRANSFER_SRC);
        match renderer.add_swapchain(&self.gpu, swapchain, &images, info.image_format, info.image_extent, transfer) {
            Ok(()) => vk::Result::SUCCESS,
            Err(e) => e,
        }
    }
}

unsafe extern "system" fn queue_present(queue: vk::Queue, info: *const vk::PresentInfoKHR) -> vk::Result {
    let Some(data) = device_data(key(queue)) else { return vk::Result::ERROR_DEVICE_LOST };
    let mut data = data.lock().unwrap();
    let present = data.swapchain_fn.queue_present_khr;
    let info = &*info;
    let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| data.draw(queue, info))).unwrap_or_else(|_| {
        crate::log("overlay crashed; disabled for this device");
        data.renderer = None;
        None
    });
    drop(data);
    match drawn {
        Some(semaphore) => {
            let wait = [semaphore];
            let mut info = *info;
            info.wait_semaphore_count = 1;
            info.p_wait_semaphores = wait.as_ptr();
            present(queue, &info)
        }
        None => present(queue, info),
    }
}

impl DeviceData {
    /// Draws the overlay on the first swapchain of the present.
    unsafe fn draw(&mut self, queue: vk::Queue, info: &vk::PresentInfoKHR) -> Option<vk::Semaphore> {
        let state = self.client.poll()?.clone();
        let &(family, graphics) = self.queues.get(&queue.as_raw())?;
        if !graphics || info.swapchain_count == 0 {
            return None;
        }
        let swapchain = *info.p_swapchains;
        let image = *info.p_image_indices;
        let wait = std::slice::from_raw_parts(info.p_wait_semaphores, info.wait_semaphore_count as usize);
        let renderer = self.renderer.as_mut()?;
        match renderer.draw(&self.gpu, queue, family, swapchain, image, wait, &state, &mut self.client) {
            Ok(semaphore) => semaphore,
            Err(e) => {
                crate::log(&format!("overlay draw failed: {e}"));
                None
            }
        }
    }
}
