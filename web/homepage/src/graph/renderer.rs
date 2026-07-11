use glam::{Mat4, Vec3};
use web_sys::{
    WebGlBuffer, WebGlProgram, WebGlRenderingContext as GL, WebGlShader, WebGlTexture,
    WebGlUniformLocation,
};

use super::camera::Camera;
use super::data::GraphData;
use super::simulation::Simulation;

// Shader sources
const SPRITE_VERT: &str = r#"
    attribute vec2 a_quad;
    uniform vec3 u_center;
    uniform float u_size;
    uniform mat4 u_view;
    uniform mat4 u_proj;
    varying vec2 v_uv;
    void main() {
        v_uv = a_quad + 0.5;
        vec4 viewCenter = u_view * vec4(u_center, 1.0);
        vec4 viewPos = viewCenter + vec4(a_quad * u_size, 0.0, 0.0);
        gl_Position = u_proj * viewPos;
    }
"#;

const SPRITE_FRAG: &str = r#"
    precision mediump float;
    varying vec2 v_uv;
    uniform sampler2D u_texture;
    uniform float u_circle;
    void main() {
        vec4 color = texture2D(u_texture, v_uv);
        // Alpha cutout: only solid texels draw (and write depth), so icons
        // occlude by true depth rather than draw order.
        if (color.a < 0.5) discard;
        // Optionally clip to the inscribed circle so square textures (atproto
        // badges, the avatar photo) render as round icons.
        if (u_circle > 0.5) {
            vec2 d = v_uv - vec2(0.5);
            if (dot(d, d) > 0.25) discard;
        }
        gl_FragColor = color;
    }
"#;

const SPHERE_VERT: &str = r#"
    attribute vec3 a_position;
    attribute vec3 a_normal;
    uniform mat4 u_model;
    uniform mat4 u_view;
    uniform mat4 u_proj;
    varying vec3 v_normal;
    varying vec3 v_worldPos;
    void main() {
        vec4 worldPos = u_model * vec4(a_position, 1.0);
        v_worldPos = worldPos.xyz;
        v_normal = mat3(u_model) * a_normal;
        gl_Position = u_proj * u_view * worldPos;
    }
"#;

const SPHERE_FRAG: &str = r#"
    precision mediump float;
    varying vec3 v_normal;
    varying vec3 v_worldPos;
    uniform vec3 u_color;
    uniform float u_opacity;
    uniform vec3 u_lightDir;
    void main() {
        vec3 n = normalize(v_normal);
        float diff = max(dot(n, normalize(u_lightDir)), 0.0);
        float ambient = 0.3;
        vec3 lit = u_color * (ambient + diff * 0.7);
        gl_FragColor = vec4(lit, u_opacity);
    }
"#;

const LINE_VERT: &str = r#"
    attribute vec3 a_position;
    uniform mat4 u_view;
    uniform mat4 u_proj;
    void main() {
        gl_Position = u_proj * u_view * vec4(a_position, 1.0);
    }
"#;

const LINE_FRAG: &str = r#"
    precision mediump float;
    uniform vec4 u_color;
    void main() {
        gl_FragColor = u_color;
    }
"#;

const PARTICLE_VERT: &str = r#"
    attribute vec2 a_quad;
    uniform vec3 u_center;
    uniform float u_size;
    uniform mat4 u_view;
    uniform mat4 u_proj;
    void main() {
        vec4 viewCenter = u_view * vec4(u_center, 1.0);
        vec4 viewPos = viewCenter + vec4(a_quad * u_size, 0.0, 0.0);
        gl_Position = u_proj * viewPos;
    }
"#;

const PARTICLE_FRAG: &str = r#"
    precision mediump float;
    void main() {
        gl_FragColor = vec4(1.0, 1.0, 1.0, 0.6);
    }
"#;

struct ShaderProgram {
    program: WebGlProgram,
}

impl ShaderProgram {
    fn get_uniform(&self, gl: &GL, name: &str) -> Option<WebGlUniformLocation> {
        gl.get_uniform_location(&self.program, name)
    }

    fn get_attrib(&self, gl: &GL, name: &str) -> u32 {
        gl.get_attrib_location(&self.program, name) as u32
    }
}

// Sphere mesh data
struct SphereMesh {
    vertex_buffer: WebGlBuffer,
    index_buffer: WebGlBuffer,
    index_count: i32,
}

pub struct Renderer {
    gl: GL,
    sprite_program: ShaderProgram,
    sphere_program: ShaderProgram,
    line_program: ShaderProgram,
    particle_program: ShaderProgram,
    quad_buffer: WebGlBuffer,
    quad_index_buffer: WebGlBuffer,
    sphere_mesh: SphereMesh,
    line_buffer: WebGlBuffer,
}

impl Renderer {
    pub fn new(gl: GL) -> Result<Self, String> {
        // Enable blending and depth test
        gl.enable(GL::BLEND);
        gl.blend_func(GL::SRC_ALPHA, GL::ONE_MINUS_SRC_ALPHA);
        gl.enable(GL::DEPTH_TEST);
        gl.clear_color(0.133, 0.133, 0.133, 1.0); // #222222

        // Compile shader programs
        let sprite_program = compile_program(&gl, SPRITE_VERT, SPRITE_FRAG)?;
        let sphere_program = compile_program(&gl, SPHERE_VERT, SPHERE_FRAG)?;
        let line_program = compile_program(&gl, LINE_VERT, LINE_FRAG)?;
        let particle_program = compile_program(&gl, PARTICLE_VERT, PARTICLE_FRAG)?;

        // Create quad geometry (unit quad centered at origin)
        let quad_verts: [f32; 8] = [-0.5, -0.5, 0.5, -0.5, 0.5, 0.5, -0.5, 0.5];
        let quad_indices: [u16; 6] = [0, 1, 2, 0, 2, 3];
        let quad_buffer = create_buffer(&gl, GL::ARRAY_BUFFER, &quad_verts)?;
        let quad_index_buffer = create_buffer(&gl, GL::ELEMENT_ARRAY_BUFFER, &quad_indices)?;

        // Create sphere geometry
        let sphere_mesh = create_sphere_mesh(&gl, 15.0, 16, 12)?;

        // Create line buffer (dynamic, will be updated each frame)
        let line_buffer = gl.create_buffer().ok_or("failed to create line buffer")?;

        Ok(Self {
            gl,
            sprite_program,
            sphere_program,
            line_program,
            particle_program,
            quad_buffer,
            quad_index_buffer,
            sphere_mesh,
            line_buffer,
        })
    }

    pub fn resize(&self, width: u32, height: u32) {
        self.gl.viewport(0, 0, width as i32, height as i32);
    }

    pub fn render(
        &self,
        camera: &Camera,
        simulation: &Simulation,
        data: &GraphData,
        textures: &std::collections::HashMap<String, WebGlTexture>,
        particle_positions: &[(Vec3, Vec3)], // pairs of particle positions per link
    ) {
        let gl = &self.gl;
        gl.clear(GL::COLOR_BUFFER_BIT | GL::DEPTH_BUFFER_BIT);

        let view = camera.view_matrix();
        let proj = camera.projection_matrix();

        // Pass 1: opaque icon sprites, depth writing ON. The sprite shader
        // discards transparent texels (alpha cutout), so icons occlude one
        // another by true depth instead of draw order. This is what keeps the
        // central node from hiding behind everything.
        gl.depth_mask(true);
        for (i, node) in data.nodes.iter().enumerate() {
            let size = node_size(node);
            if let Some(tex) = textures.get(&node.icon) {
                self.draw_sprite(
                    gl,
                    &view,
                    &proj,
                    simulation.positions[i],
                    size,
                    tex,
                    data.circular_icons,
                );
            }
        }

        // Pass 2: translucent elements, depth writing OFF so they blend against
        // the opaque icons without corrupting the depth buffer. They are still
        // depth-tested, so links/halos correctly disappear behind nearer nodes.
        gl.depth_mask(false);

        // Links and their flow particles sit behind the nodes.
        self.draw_links(gl, &view, &proj, data, &simulation.positions);
        self.draw_particles(gl, &view, &proj, particle_positions);

        // Colored halo spheres, drawn back-to-front (farthest first in view
        // space) so overlapping translucency composites correctly.
        let mut halos: Vec<(usize, f32, &str)> = data
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let color = n.color.as_deref()?;
                let view_z = (view * simulation.positions[i].extend(1.0)).z;
                Some((i, view_z, color))
            })
            .collect();
        halos.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (i, _, color_str) in halos {
            let color = parse_hex_color(color_str);
            let opacity = data.nodes[i].opacity.unwrap_or(0.4);
            self.draw_sphere(gl, &view, &proj, simulation.positions[i], color, opacity);
        }

        gl.depth_mask(true);
    }

    fn draw_links(&self, gl: &GL, view: &Mat4, proj: &Mat4, data: &GraphData, positions: &[Vec3]) {
        // Build line vertex data
        let mut line_data: Vec<f32> = Vec::with_capacity(data.links.len() * 6);
        for link in &data.links {
            let si = data.node_index(&link.source);
            let ti = data.node_index(&link.target);
            if let (Some(si), Some(ti)) = (si, ti) {
                let s = positions[si];
                let t = positions[ti];
                line_data.extend_from_slice(&[s.x, s.y, s.z, t.x, t.y, t.z]);
            }
        }

        if line_data.is_empty() {
            return;
        }

        gl.use_program(Some(&self.line_program.program));

        // Upload line data
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&self.line_buffer));
        unsafe {
            let array = js_sys::Float32Array::view(&line_data);
            gl.buffer_data_with_array_buffer_view(GL::ARRAY_BUFFER, &array, GL::DYNAMIC_DRAW);
        }

        let a_pos = self.line_program.get_attrib(gl, "a_position");
        gl.enable_vertex_attrib_array(a_pos);
        gl.vertex_attrib_pointer_with_i32(a_pos, 3, GL::FLOAT, false, 0, 0);

        // Set uniforms
        gl.uniform_matrix4fv_with_f32_array(
            self.line_program.get_uniform(gl, "u_view").as_ref(),
            false,
            &view.to_cols_array(),
        );
        gl.uniform_matrix4fv_with_f32_array(
            self.line_program.get_uniform(gl, "u_proj").as_ref(),
            false,
            &proj.to_cols_array(),
        );
        gl.uniform4f(
            self.line_program.get_uniform(gl, "u_color").as_ref(),
            1.0,
            1.0,
            1.0,
            0.2,
        );

        gl.draw_arrays(GL::LINES, 0, (line_data.len() / 3) as i32);
        gl.disable_vertex_attrib_array(a_pos);
    }

    fn draw_particles(
        &self,
        gl: &GL,
        view: &Mat4,
        proj: &Mat4,
        particle_positions: &[(Vec3, Vec3)],
    ) {
        gl.use_program(Some(&self.particle_program.program));

        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&self.quad_buffer));
        let a_quad = self.particle_program.get_attrib(gl, "a_quad");
        gl.enable_vertex_attrib_array(a_quad);
        gl.vertex_attrib_pointer_with_i32(a_quad, 2, GL::FLOAT, false, 0, 0);

        gl.bind_buffer(GL::ELEMENT_ARRAY_BUFFER, Some(&self.quad_index_buffer));

        gl.uniform_matrix4fv_with_f32_array(
            self.particle_program.get_uniform(gl, "u_view").as_ref(),
            false,
            &view.to_cols_array(),
        );
        gl.uniform_matrix4fv_with_f32_array(
            self.particle_program.get_uniform(gl, "u_proj").as_ref(),
            false,
            &proj.to_cols_array(),
        );
        gl.uniform1f(
            self.particle_program.get_uniform(gl, "u_size").as_ref(),
            1.5,
        );

        for (p1, p2) in particle_positions {
            for p in [p1, p2] {
                gl.uniform3f(
                    self.particle_program.get_uniform(gl, "u_center").as_ref(),
                    p.x,
                    p.y,
                    p.z,
                );
                gl.draw_elements_with_i32(GL::TRIANGLES, 6, GL::UNSIGNED_SHORT, 0);
            }
        }

        gl.disable_vertex_attrib_array(a_quad);
    }

    fn draw_sphere(&self, gl: &GL, view: &Mat4, proj: &Mat4, pos: Vec3, color: Vec3, opacity: f32) {
        gl.use_program(Some(&self.sphere_program.program));

        // Bind sphere mesh
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&self.sphere_mesh.vertex_buffer));
        let a_pos = self.sphere_program.get_attrib(gl, "a_position");
        let a_normal = self.sphere_program.get_attrib(gl, "a_normal");
        gl.enable_vertex_attrib_array(a_pos);
        gl.enable_vertex_attrib_array(a_normal);
        // Stride: 6 floats (3 pos + 3 normal) * 4 bytes = 24
        gl.vertex_attrib_pointer_with_i32(a_pos, 3, GL::FLOAT, false, 24, 0);
        gl.vertex_attrib_pointer_with_i32(a_normal, 3, GL::FLOAT, false, 24, 12);

        gl.bind_buffer(
            GL::ELEMENT_ARRAY_BUFFER,
            Some(&self.sphere_mesh.index_buffer),
        );

        let model = Mat4::from_translation(pos);
        gl.uniform_matrix4fv_with_f32_array(
            self.sphere_program.get_uniform(gl, "u_model").as_ref(),
            false,
            &model.to_cols_array(),
        );
        gl.uniform_matrix4fv_with_f32_array(
            self.sphere_program.get_uniform(gl, "u_view").as_ref(),
            false,
            &view.to_cols_array(),
        );
        gl.uniform_matrix4fv_with_f32_array(
            self.sphere_program.get_uniform(gl, "u_proj").as_ref(),
            false,
            &proj.to_cols_array(),
        );
        gl.uniform3f(
            self.sphere_program.get_uniform(gl, "u_color").as_ref(),
            color.x,
            color.y,
            color.z,
        );
        gl.uniform1f(
            self.sphere_program.get_uniform(gl, "u_opacity").as_ref(),
            opacity,
        );
        gl.uniform3f(
            self.sphere_program.get_uniform(gl, "u_lightDir").as_ref(),
            0.5,
            1.0,
            0.3,
        );

        gl.draw_elements_with_i32(
            GL::TRIANGLES,
            self.sphere_mesh.index_count,
            GL::UNSIGNED_SHORT,
            0,
        );

        gl.disable_vertex_attrib_array(a_pos);
        gl.disable_vertex_attrib_array(a_normal);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_sprite(
        &self,
        gl: &GL,
        view: &Mat4,
        proj: &Mat4,
        pos: Vec3,
        size: f32,
        texture: &WebGlTexture,
        circle: bool,
    ) {
        gl.use_program(Some(&self.sprite_program.program));

        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&self.quad_buffer));
        let a_quad = self.sprite_program.get_attrib(gl, "a_quad");
        gl.enable_vertex_attrib_array(a_quad);
        gl.vertex_attrib_pointer_with_i32(a_quad, 2, GL::FLOAT, false, 0, 0);

        gl.bind_buffer(GL::ELEMENT_ARRAY_BUFFER, Some(&self.quad_index_buffer));

        gl.uniform3f(
            self.sprite_program.get_uniform(gl, "u_center").as_ref(),
            pos.x,
            pos.y,
            pos.z,
        );
        gl.uniform1f(self.sprite_program.get_uniform(gl, "u_size").as_ref(), size);
        gl.uniform1f(
            self.sprite_program.get_uniform(gl, "u_circle").as_ref(),
            if circle { 1.0 } else { 0.0 },
        );
        gl.uniform_matrix4fv_with_f32_array(
            self.sprite_program.get_uniform(gl, "u_view").as_ref(),
            false,
            &view.to_cols_array(),
        );
        gl.uniform_matrix4fv_with_f32_array(
            self.sprite_program.get_uniform(gl, "u_proj").as_ref(),
            false,
            &proj.to_cols_array(),
        );

        // Bind texture
        gl.active_texture(GL::TEXTURE0);
        gl.bind_texture(GL::TEXTURE_2D, Some(texture));
        gl.uniform1i(self.sprite_program.get_uniform(gl, "u_texture").as_ref(), 0);

        gl.draw_elements_with_i32(GL::TRIANGLES, 6, GL::UNSIGNED_SHORT, 0);
        gl.disable_vertex_attrib_array(a_quad);
    }
}

fn compile_shader(gl: &GL, shader_type: u32, source: &str) -> Result<WebGlShader, String> {
    let shader = gl
        .create_shader(shader_type)
        .ok_or("failed to create shader")?;
    gl.shader_source(&shader, source);
    gl.compile_shader(&shader);

    if !gl
        .get_shader_parameter(&shader, GL::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        let info = gl.get_shader_info_log(&shader).unwrap_or_default();
        gl.delete_shader(Some(&shader));
        return Err(format!("shader compilation failed: {info}"));
    }
    Ok(shader)
}

fn compile_program(gl: &GL, vert_src: &str, frag_src: &str) -> Result<ShaderProgram, String> {
    let vert = compile_shader(gl, GL::VERTEX_SHADER, vert_src)?;
    let frag = compile_shader(gl, GL::FRAGMENT_SHADER, frag_src)?;

    let program = gl.create_program().ok_or("failed to create program")?;
    gl.attach_shader(&program, &vert);
    gl.attach_shader(&program, &frag);
    gl.link_program(&program);

    if !gl
        .get_program_parameter(&program, GL::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        let info = gl.get_program_info_log(&program).unwrap_or_default();
        gl.delete_program(Some(&program));
        return Err(format!("program link failed: {info}"));
    }

    gl.delete_shader(Some(&vert));
    gl.delete_shader(Some(&frag));

    Ok(ShaderProgram { program })
}

fn create_buffer<T: bytemuck_compatible::Pod>(
    gl: &GL,
    target: u32,
    data: &[T],
) -> Result<WebGlBuffer, String> {
    let buffer = gl.create_buffer().ok_or("failed to create buffer")?;
    gl.bind_buffer(target, Some(&buffer));
    unsafe {
        let byte_slice =
            std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data));
        let array = js_sys::Uint8Array::view(byte_slice);
        gl.buffer_data_with_array_buffer_view(target, &array, GL::STATIC_DRAW);
    }
    Ok(buffer)
}

fn create_sphere_mesh(
    gl: &GL,
    radius: f32,
    segments: u32,
    rings: u32,
) -> Result<SphereMesh, String> {
    let mut vertices: Vec<f32> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();

    for y in 0..=rings {
        let phi = std::f32::consts::PI * y as f32 / rings as f32;
        for x in 0..=segments {
            let theta = 2.0 * std::f32::consts::PI * x as f32 / segments as f32;

            let nx = phi.sin() * theta.cos();
            let ny = phi.cos();
            let nz = phi.sin() * theta.sin();

            vertices.extend_from_slice(&[nx * radius, ny * radius, nz * radius, nx, ny, nz]);
        }
    }

    for y in 0..rings {
        for x in 0..segments {
            let first = y * (segments + 1) + x;
            let second = first + segments + 1;
            indices.push(first as u16);
            indices.push(second as u16);
            indices.push((first + 1) as u16);
            indices.push(second as u16);
            indices.push((second + 1) as u16);
            indices.push((first + 1) as u16);
        }
    }

    let index_count = indices.len() as i32;
    let vertex_buffer = create_buffer(gl, GL::ARRAY_BUFFER, &vertices)?;
    let index_buffer = create_buffer(gl, GL::ELEMENT_ARRAY_BUFFER, &indices)?;

    Ok(SphereMesh {
        vertex_buffer,
        index_buffer,
        index_count,
    })
}

fn node_size(node: &super::data::GraphNode) -> f32 {
    if node.center {
        40.0
    } else if node.color.is_some() {
        20.0
    } else {
        18.0
    }
}

fn parse_hex_color(hex: &str) -> Vec3 {
    let hex = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
    Vec3::new(r, g, b)
}

mod bytemuck_compatible {
    /// # Safety
    /// Implementors must be plain-old-data types (no padding, no pointers).
    pub unsafe trait Pod: Copy + 'static {}
    unsafe impl Pod for f32 {}
    unsafe impl Pod for u16 {}
}
