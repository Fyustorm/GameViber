//! Linux: the implicit Vulkan layer (`layer`, `render`), the OpenGL swap hooks
//! when preloaded (`gl`), the socket client and the shared frame memory (a
//! sealed memfd, `capture`).

mod capture;
mod client;
mod gl;
mod layer;
mod render;
