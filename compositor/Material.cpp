// Compositor-side liquid glass for enzui-material-v1, using the pinned framework shaders.
#include "src/plugins/PluginAPI.hpp"
#include "src/Compositor.hpp"
#include "src/desktop/view/window/Window.hpp"
#include "src/desktop/view/window/WindowPresentation.hpp"
#include "src/state/MonitorState.hpp"
#include "src/workspace/HLWorkspace.hpp"
#include "src/desktop/view/LayerSurface.hpp"
#include "src/desktop/view/WLSurface.hpp"
#include "src/desktop/rule/windowRule/WindowRuleApplicator.hpp"
#include "src/output/Monitor.hpp"
#include "src/protocols/core/Compositor.hpp"
#include "src/render/Renderer.hpp"
#include "src/render/OpenGL.hpp"
#include "src/render/gl/GLFramebuffer.hpp"
#include "src/render/pass/PassElement.hpp"
#include <GLES3/gl32.h>
#include "material-shaders.h"
#include "MaterialReadback.hpp"
#include <wayland-server-core.h>
#include <algorithm>
#include <any>
#include <array>
#include <cmath>
#include <cstring>
#include <map>
#include <optional>
#include <memory>
#include <stdexcept>
#include <vector>

#include <dlfcn.h>

extern "C" {
#include "enzui-material-v1-protocol.h"
}

namespace EnzMaterial {
// The compositor's globals, resolved from the executable: a hidden plugin
// copy of an inline global would be null.
static UP<CCompositor>* compositorSlot = nullptr;
static UP<Render::IHyprRenderer>* rendererSlot = nullptr;
static CCompositor& compositor() { return **compositorSlot; }
static Render::IHyprRenderer& renderer() { return **rendererSlot; }
}

namespace EnzMaterial {

constexpr size_t REGION_FLOATS = 25;
constexpr size_t MAX_REGIONS = 32;
// The framework's own limits (iced_wgpu material.rs), so the same tokens
// spend the same filter.
constexpr float MAX_BLUR_SIGMA = 32.0F;
constexpr float MAX_FILTER_RADIUS = 64.0F;
constexpr float MIN_FILTER_REACH = 1.0F;

struct Tokens {
    float blur_sigma, refraction, bevel_width, index_of_refraction, dispersion, color_diffusion, color_reach, optical_depth, tint_opacity, saturation,
        highlight, brightness, light_angle, profile_curvature, contrast_protection, protected_inset;
};
struct Region {
    float x, y, w, h, radius;
    Tokens t;
    float tint[4];
};
static_assert(sizeof(Region) == REGION_FLOATS * sizeof(float));

struct Shadow {
    float blur = 0, offset = 0, opacity = 0;
};
// Where the surface covers its glass with opaque content (version 3).
constexpr size_t CUTOUT_FLOATS = 5;
struct Cutout {
    float x, y, w, h, radius;
};
static_assert(sizeof(Cutout) == CUTOUT_FLOATS * sizeof(float));
struct Material {
    wl_resource* resource = nullptr;
    WP<CWLSurfaceResource> surface;
    std::vector<Region> pending;
    std::vector<Region> current;
    Shadow pendingShadow, currentShadow;
    std::vector<Cutout> pendingCutouts, currentCutouts;
    // The surface's own damage since the last frame, global logical: what the
    // window drew changes what is on screen but never its glass's backdrop.
    CRegion ownDamage;
    bool pendingMeasureOnly = false, currentMeasureOnly = false;
    // The luminance last sent for each region; negative before the first.
    std::vector<float> sent;
    bool dirty = false;
    ReadbackPolicy::Stamp stamp;
    bool readbackDisabled = false;
    // Whether the backdrop may have changed since the regions were last
    // measured, and where the window was then: a reading depends only on the
    // backdrop, so the window's own repaints never ask for another.
    bool unmeasured = true;
    Vector2D measuredAt{-1e9, -1e9};
    // Whether the client ever waited for a reading (asked to measure only):
    // a client that never does has no use for luminance.
    bool wantsReadings = false;
    Vector2D committedSize;
    CHyprSignalListener precommit;
    CHyprSignalListener commit;
    CHyprSignalListener destroyed;
};

static HANDLE owner = nullptr;
static wl_global* global = nullptr;
static std::map<wl_resource*, std::unique_ptr<Material>> materials;
static uint64_t nextMaterialLifetime = 1;
static std::vector<wl_resource*> managers;
static CHyprSignalListener stageListener;
static CHyprSignalListener checksListener;
// Windows whose glass this frame already holds: a window can be rendered in a
// main and a popup pass, and only the first comes before its content.
static std::vector<const void*> glazed;

static Material* forSurface(CWLSurfaceResource* surface) {
    for (auto& [_, m] : materials)
        if (m->surface.get() == surface)
            return m.get();
    return nullptr;
}


static void materialDestroy(wl_client*, wl_resource* resource) { wl_resource_destroy(resource); }

static bool finite(const Region& r) {
    const float* v = reinterpret_cast<const float*>(&r);
    for (size_t i = 0; i < REGION_FLOATS; ++i)
        if (!std::isfinite(v[i]))
            return false;
    return r.w >= 0 && r.h >= 0 && r.radius >= 0;
}

static void materialSetRegions(wl_client*, wl_resource* resource, wl_array* array) {
    auto it = materials.find(resource);
    if (it == materials.end())
        return;
    if (array->size % (REGION_FLOATS * sizeof(float)) != 0 || array->size / (REGION_FLOATS * sizeof(float)) > MAX_REGIONS) {
        wl_resource_post_error(resource, ENZUI_SURFACE_MATERIAL_V1_ERROR_INVALID_REGIONS, "regions must be whole 25-float records, at most 32");
        return;
    }
    std::vector<Region> regions(array->size / sizeof(Region));
    std::memcpy(regions.data(), array->data, array->size);
    for (const auto& r : regions)
        if (!finite(r)) {
            wl_resource_post_error(resource, ENZUI_SURFACE_MATERIAL_V1_ERROR_INVALID_REGIONS, "a region is not finite or has a negative size");
            return;
        }
    it->second->pending = std::move(regions);
    it->second->dirty = true;
}

static void materialSetShadow(wl_client*, wl_resource* resource, wl_fixed_t blur, wl_fixed_t offset, wl_fixed_t opacity) {
    auto it = materials.find(resource);
    if (it == materials.end())
        return;
    it->second->pendingShadow = {std::clamp(float(wl_fixed_to_double(blur)), 0.F, 64.F), std::clamp(float(wl_fixed_to_double(offset)), -64.F, 64.F),
                                 std::clamp(float(wl_fixed_to_double(opacity)), 0.F, 1.F)};
    it->second->dirty = true;
}

static void materialSetMeasureOnly(wl_client*, wl_resource* resource, uint32_t measureOnly) {
    auto it = materials.find(resource);
    if (it == materials.end())
        return;
    it->second->pendingMeasureOnly = measureOnly != 0;
    it->second->dirty = true;
}

static void materialSetCutouts(wl_client*, wl_resource* resource, wl_array* array) {
    auto it = materials.find(resource);
    if (it == materials.end())
        return;
    if (array->size % (CUTOUT_FLOATS * sizeof(float)) != 0 || array->size / (CUTOUT_FLOATS * sizeof(float)) > MAX_REGIONS) {
        wl_resource_post_error(resource, ENZUI_SURFACE_MATERIAL_V1_ERROR_INVALID_CUTOUTS, "cutouts must be whole 5-float records, at most 32");
        return;
    }
    std::vector<Cutout> cutouts(array->size / sizeof(Cutout));
    std::memcpy(cutouts.data(), array->data, array->size);
    for (const auto& c : cutouts)
        if (!std::isfinite(c.x) || !std::isfinite(c.y) || !std::isfinite(c.w) || !std::isfinite(c.h) || !std::isfinite(c.radius) || c.w < 0 || c.h < 0 ||
            c.radius < 0) {
            wl_resource_post_error(resource, ENZUI_SURFACE_MATERIAL_V1_ERROR_INVALID_CUTOUTS, "a cutout is not finite or has a negative size");
            return;
        }
    it->second->pendingCutouts = std::move(cutouts);
    it->second->dirty = true;
}

static const struct enzui_surface_material_v1_interface materialImpl = {
    .destroy = materialDestroy,
    .set_regions = materialSetRegions,
    .set_shadow = materialSetShadow,
    .set_measure_only = materialSetMeasureOnly,
    .set_cutouts = materialSetCutouts,
};

// Match the material shader's 0.75-pixel antialias band in damage and cache bounds.
static CBox glassRasterBox(const CBox& box) {
    constexpr double AA = 0.75;
    const double x = std::floor(box.x - AA), y = std::floor(box.y - AA);
    return {x, y, std::ceil(box.x + box.w + AA) - x, std::ceil(box.y + box.h + AA) - y};
}

// The glass a surface shows, in surface-local logical pixels: its regions
// less what its cutouts cover. A cutout covers its rounded rectangle less the
// corners outside its radius, a pixel short of every edge.
// The glass the cutouts leave visible, in framebuffer texels: a surface-local
// logical point p lands at (at + p * stretch) * scale. Glass reaches outward
// to every texel it touches; a cutout hides only the texels its opaque cross
// (the rect without its rounded corners) covers whole, so a pane edge between
// texels, or its corner arcs, keep the glass beneath them. A pane edge on a
// texel boundary hides everything under it, so the window repainting it never
// touches the glass beside it.
static CRegion visibleGlass(const std::vector<Region>& regions, const std::vector<Cutout>& cutouts, Vector2D at, Vector2D stretch, double scale) {
    auto texels = [&](double x, double y, double w, double h, bool outward) {
        const double x0 = (at.x + x * stretch.x) * scale, x1 = (at.x + (x + w) * stretch.x) * scale;
        const double y0 = (at.y + y * stretch.y) * scale, y1 = (at.y + (y + h) * stretch.y) * scale;
        if (outward)
            return glassRasterBox({x0, y0, x1 - x0, y1 - y0});
        constexpr double EPS = 1e-3;
        const double l = std::ceil(x0 - EPS), r = std::floor(x1 + EPS);
        const double t = std::ceil(y0 - EPS), b = std::floor(y1 + EPS);
        return CBox{l, t, std::max(0.0, r - l), std::max(0.0, b - t)};
    };
    CRegion visible;
    for (const auto& r : regions)
        if (r.w > 0 && r.h > 0)
            visible.add(texels(r.x, r.y, r.w, r.h, true));
    for (const auto& c : cutouts) {
        const float radius = std::clamp(c.radius, 0.F, std::min(c.w, c.h) / 2.F);
        CRegion cross;
        if (c.w > 2 * radius && c.h > 0)
            cross.add(texels(c.x + radius, c.y, c.w - 2 * radius, c.h, false));
        if (c.h > 2 * radius && c.w > 0)
            cross.add(texels(c.x, c.y + radius, c.w, c.h - 2 * radius, false));
        visible.subtract(cross);
    }
    return visible;
}

static float samplePadding(const Region& r);

// Damage includes old and new filter/shadow extents; unplaced surfaces damage every output.
static void damageSurface(Material* m, const std::vector<Region>& before, const Shadow& beforeShadow) {
    std::optional<CBox> bounds;
    float reach = 0;
    for (const auto* set : std::array<const std::vector<Region>*, 2>{&before, &m->current})
        for (const auto& r : *set) {
            const CBox box = {r.x, r.y, r.w, r.h};
            if (!bounds) {
                bounds = box;
            } else {
                const double x0 = std::min(bounds->x, box.x), y0 = std::min(bounds->y, box.y);
                const double x1 = std::max(bounds->x + bounds->width, box.x + box.width), y1 = std::max(bounds->y + bounds->height, box.y + box.height);
                bounds = CBox{x0, y0, x1 - x0, y1 - y0};
            }
            reach = std::max(reach, samplePadding(r));
        }
    for (const auto* shadow : std::array<const Shadow*, 2>{&beforeShadow, &m->currentShadow})
        reach = std::max(reach, shadow->blur * 1.5F + std::abs(shadow->offset));
    const auto surface = m->surface.lock();
    const auto view = surface ? Desktop::View::CWLSurface::fromResource(surface) : nullptr;
    const auto window = view ? Desktop::View::CWindow::fromView(view->view()) : nullptr;
    if (!window) {
        for (auto const& mon : State::monitorState()->monitors())
            renderer().damageMonitor(mon);
        return;
    }
    // Hyprland's own blur would replace the backdrop behind the glass with
    // its blurred copy, so a window with glass is never blurred by it.
    if (m->current.empty())
        window->m_ruleApplicator->noBlur().unset(Desktop::Types::PRIORITY_SET_PROP);
    else
        window->m_ruleApplicator->noBlur().set(true, Desktop::Types::PRIORITY_SET_PROP);
    if (!bounds)
        return;
    const auto geometry = window->backend().geometry().box;
    const auto origin = window->position(Desktop::View::IGeometric::GEOMETRIC_CURRENT) - Vector2D{geometry.x, geometry.y};
    renderer().damageBox(bounds->copy().translate(origin).expand(reach + 2));
}

static void retireCache(const void* material);

static void materialResourceDestroyed(wl_resource* resource) {
    auto it = materials.find(resource);
    if (it == materials.end())
        return;
    const auto previous = it->second->current;
    const auto previousShadow = it->second->currentShadow;
    it->second->current.clear();
    damageSurface(it->second.get(), previous, previousShadow);
    retireCache(it->second.get());
    materials.erase(it);
}

static void managerDestroy(wl_client*, wl_resource* resource) { wl_resource_destroy(resource); }

static void managerGetSurfaceMaterial(wl_client* client, wl_resource* manager, uint32_t id, wl_resource* surfaceResource) {
    auto surface = CWLSurfaceResource::fromResource(surfaceResource);
    if (!surface) {
        wl_resource_post_error(manager, ENZUI_MATERIAL_MANAGER_V1_ERROR_ALREADY_CONSTRUCTED, "not a surface");
        return;
    }
    if (forSurface(surface.get())) {
        wl_resource_post_error(manager, ENZUI_MATERIAL_MANAGER_V1_ERROR_ALREADY_CONSTRUCTED, "the surface already has a material object");
        return;
    }
    auto resource = wl_resource_create(client, &enzui_surface_material_v1_interface, wl_resource_get_version(manager), id);
    if (!resource) {
        wl_client_post_no_memory(client);
        return;
    }
    auto material = std::make_unique<Material>();
    material->resource = resource;
    material->surface = surface;
    material->stamp.lifetime = nextMaterialLifetime++;
    material->committedSize = surface->m_current.size;
    Material* raw = material.get();
    // The damage a commit brings, read from the pending state before the
    // compositor takes it (reading the current state would consume it).
    material->precommit = surface->m_events.precommit.listen([raw] {
        const auto surface = raw->surface.lock();
        const auto view = surface ? Desktop::View::CWLSurface::fromResource(surface) : nullptr;
        const auto box = view ? view->getSurfaceBoxGlobal() : std::nullopt;
        if (!box)
            return;
        const auto& state = surface->m_pending;
        CRegion damage = state.damage.copy();
        if (!state.bufferDamage.empty()) {
            // A plain buffer maps to the surface by its scale; anything else
            // counts as damage everywhere.
            if (state.transform == WL_OUTPUT_TRANSFORM_NORMAL && !state.viewport.hasSource && !state.viewport.hasDestination && state.scale > 0)
                damage.add(state.bufferDamage.copy().scale(1.0 / state.scale));
            else
                damage.add(CBox{{}, box->size()});
        }
        damage.intersect(CBox{{}, box->size()});
        raw->ownDamage.add(damage.translate(box->pos()));
    });
    material->commit = surface->m_events.commit.listen([raw] {
        const auto surface = raw->surface.lock();
        const auto size = surface ? surface->m_current.size : Vector2D{};
        const bool resized = size != raw->committedSize;
        if (!raw->dirty && !resized)
            return;
        const auto previous = raw->current;
        const auto previousShadow = raw->currentShadow;
        raw->current = raw->pending;
        raw->currentShadow = raw->pendingShadow;
        raw->currentCutouts = raw->pendingCutouts;
        raw->currentMeasureOnly = raw->pendingMeasureOnly;
        raw->wantsReadings |= raw->currentMeasureOnly;
        ++raw->stamp.revision;
        raw->committedSize = size;
        raw->sent.assign(raw->current.size(), -1.F);
        raw->dirty = false;
        raw->unmeasured = true;
        damageSurface(raw, previous, previousShadow);
    });
    material->destroyed = surface->m_events.destroy.listen([raw] {
        ++raw->stamp.revision;
        raw->current.clear();
        raw->surface.reset();
        raw->commit.reset();
        raw->destroyed.reset();
    });
    materials.emplace(resource, std::move(material));
    wl_resource_set_implementation(resource, &materialImpl, nullptr, materialResourceDestroyed);
}

static const struct enzui_material_manager_v1_interface managerImpl = {
    .destroy = managerDestroy,
    .get_surface_material = managerGetSurfaceMaterial,
};

static void bindManager(wl_client* client, void*, uint32_t version, uint32_t id) {
    auto resource = wl_resource_create(client, &enzui_material_manager_v1_interface, std::min<uint32_t>(version, 3), id);
    if (!resource) {
        wl_client_post_no_memory(client);
        return;
    }
    managers.push_back(resource);
    wl_resource_set_implementation(resource, &managerImpl, nullptr, [](wl_resource* r) { std::erase(managers, r); });
}


static const char* VERTEX_FULLSCREEN = R"(#version 300 es
void main() {
    vec2 corners[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    gl_Position = vec4(corners[gl_VertexID], 0.0, 1.0);
}
)";

// The region's rectangle grown by `reach` texels (one, for the antialiased
// silhouette; the shadow's whole falloff, for the shadow).
static const char* VERTEX_REGION = R"(#version 300 es
uniform vec4 region;
uniform vec4 surface;
uniform float reach;
void main() {
    vec2 corners[6] = vec2[6](vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    vec2 texel = region.xy + corners[gl_VertexID] * (region.zw + vec2(max(reach, 1.0)));
    gl_Position = vec4(texel.x / max(surface.x, 1.0) * 2.0 - 1.0, texel.y / max(surface.y, 1.0) * 2.0 - 1.0, 0.0, 1.0);
}
)";

// A fading window's glass: its composite, drawn aside, laid over the frame at
// the window's opacity (the framework's shader has no opacity of its own).
static const char* FRAGMENT_FADE = R"(#version 300 es
precision highp float;
uniform sampler2D fade_texture;
uniform vec2 origin;
uniform float opacity;
out vec4 frag;
void main() {
    frag = texelFetch(fade_texture, ivec2(gl_FragCoord.xy - origin), 0) * opacity;
}
)";

static const char* FRAGMENT_SHADOW = R"(#version 300 es
precision highp float;
uniform vec4 region;
uniform vec4 shape;
uniform vec4 shadow;
out vec4 frag;
float sdf(vec2 p) {
    float radius = shape.x;
    vec2 q = abs(p - region.xy) - region.zw + radius;
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}
void main() {
    vec2 p = gl_FragCoord.xy;
    float outside = sdf(p);
    float d = max(sdf(p - vec2(0.0, shadow.y)), 0.0);
    // The profile of the shell's solid panels' shadow (iced's quad shadow):
    // half strength at the edge, gone by the blur. A Gaussian at full
    // strength made glass panels' shadows read larger than solid ones.
    float blur = max(shadow.x, 0.5);
    float a = shadow.z * (1.0 - smoothstep(-blur, blur, d)) * smoothstep(-0.75, 0.75, outside);
    frag = vec4(0.0, 0.0, 0.0, a);
}
)";

static GLuint compile(GLenum kind, const char* source) {
    GLuint shader = glCreateShader(kind);
    glShaderSource(shader, 1, &source, nullptr);
    glCompileShader(shader);
    GLint ok = 0;
    glGetShaderiv(shader, GL_COMPILE_STATUS, &ok);
    if (!ok) {
        char log[2048] = {};
        glGetShaderInfoLog(shader, sizeof(log), nullptr, log);
        LOG(Log::ERR, "enzui-material: shader: {}", log);
        glDeleteShader(shader);
        return 0;
    }
    return shader;
}
static GLuint link(const char* vertex, const char* fragment) {
    GLuint v = compile(GL_VERTEX_SHADER, vertex), f = compile(GL_FRAGMENT_SHADER, fragment);
    if (!v || !f)
        return 0;
    GLuint program = glCreateProgram();
    glAttachShader(program, v);
    glAttachShader(program, f);
    glLinkProgram(program);
    glDeleteShader(v);
    glDeleteShader(f);
    GLint ok = 0;
    glGetProgramiv(program, GL_LINK_STATUS, &ok);
    if (!ok) {
        char log[2048] = {};
        glGetProgramInfoLog(program, sizeof(log), nullptr, log);
        LOG(Log::ERR, "enzui-material: link: {}", log);
        glDeleteProgram(program);
        return 0;
    }
    return program;
}

// One of the framework's programs: its uniform blocks bound to the points of
// their WGSL bindings, its samplers to the units of theirs.
static GLuint linkFramework(const EnzMaterialShaders::Program& source) {
    const GLuint program = link(source.vertex, source.fragment);
    if (!program)
        return 0;
    glUseProgram(program);
    for (unsigned i = 0; i < source.blockCount; ++i)
        if (const GLuint index = glGetUniformBlockIndex(program, source.blocks[i].name); index != GL_INVALID_INDEX)
            glUniformBlockBinding(program, index, source.blocks[i].binding);
    for (unsigned i = 0; i < source.samplerCount; ++i)
        glUniform1i(glGetUniformLocation(program, source.samplers[i].name), GLint(source.samplers[i].binding));
    return program;
}

struct Target {
    GLuint texture = 0, framebuffer = 0;
    int w = 0, h = 0;
    // Grows to the largest copy seen and stays there: the regions of one frame
    // differ in shape, and sizing to each in turn would allocate every frame.
    // Sizes are whole multiples of 4, so a texture at a half or a quarter of
    // this one's size covers exactly the same area.
    void ensure(int width, int height) {
        if (texture && width <= w && height <= h)
            return;
        const int grownW = (std::max({width, w, 1}) + 3) / 4 * 4, grownH = (std::max({height, h, 1}) + 3) / 4 * 4;
        allocate(grownW, grownH);
    }
    // Exactly `width` by `height`: a reduced copy of another target.
    void ensureExact(int width, int height) {
        if (texture && width == w && height == h)
            return;
        allocate(std::max(width, 1), std::max(height, 1));
    }
    void allocate(int grownW, int grownH) {
        release();
        w = grownW;
        h = grownH;
        glGenTextures(1, &texture);
        glBindTexture(GL_TEXTURE_2D, texture);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, nullptr);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        glGenFramebuffers(1, &framebuffer);
        glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, texture, 0);
    }
    void release() {
        if (framebuffer)
            glDeleteFramebuffers(1, &framebuffer);
        if (texture)
            glDeleteTextures(1, &texture);
        texture = framebuffer = 0;
        w = h = 0;
    }
};

struct GL {
    bool tried = false;
    GLuint filter = 0, material = 0, shadow = 0, probe = 0, fade = 0, vao = 0;
    // The framework's uniform blocks: `Material` (binding 4) and `Filter` (1).
    GLuint materialBlock = 0, filterBlock = 0;
    // Source copy, filter scratch, blurred and diffused results; one texel per
    // measured region; a fading window's composite. A wide filter runs on a
    // halved or quartered copy, with its own scratch and results.
    Target backdrop, source, scratch, blurred, diffused, probes, faded, half, quarter, smallScratch, smallBlurred, smallDiffused;
    bool ready() {
        if (!tried) {
            tried = true;
            GLint program = 0;
            glGetIntegerv(GL_CURRENT_PROGRAM, &program);
            filter = linkFramework(EnzMaterialShaders::filter);
            material = linkFramework(EnzMaterialShaders::material);
            probe = linkFramework(EnzMaterialShaders::probe);
            shadow = link(VERTEX_REGION, FRAGMENT_SHADOW);
            fade = link(VERTEX_FULLSCREEN, FRAGMENT_FADE);
            glUseProgram(program);
            glGenVertexArrays(1, &vao);
            GLint previous = 0;
            glGetIntegerv(GL_UNIFORM_BUFFER_BINDING, &previous);
            glGenBuffers(1, &materialBlock);
            glBindBuffer(GL_UNIFORM_BUFFER, materialBlock);
            glBufferData(GL_UNIFORM_BUFFER, 32 * sizeof(float), nullptr, GL_DYNAMIC_DRAW);
            glGenBuffers(1, &filterBlock);
            glBindBuffer(GL_UNIFORM_BUFFER, filterBlock);
            glBufferData(GL_UNIFORM_BUFFER, 8 * sizeof(float), nullptr, GL_DYNAMIC_DRAW);
            glBindBuffer(GL_UNIFORM_BUFFER, previous);
        }
        return filter && material && shadow && probe && fade && vao && materialBlock && filterBlock;
    }
    void release() {
        for (auto* t : {&backdrop, &source, &scratch, &blurred, &diffused, &probes, &faded, &half, &quarter, &smallScratch, &smallBlurred, &smallDiffused})
            t->release();
        if (fade)
            glDeleteProgram(fade);
        for (auto* buffer : {&materialBlock, &filterBlock})
            if (*buffer)
                glDeleteBuffers(1, buffer);
        materialBlock = filterBlock = 0;
        if (probe)
            glDeleteProgram(probe);
        if (filter)
            glDeleteProgram(filter);
        if (material)
            glDeleteProgram(material);
        if (shadow)
            glDeleteProgram(shadow);
        if (vao)
            glDeleteVertexArrays(1, &vao);
        filter = material = shadow = probe = fade = vao = 0;
        tried = false;
    }
};
static GL gl;

#include "AppearanceTransition.hpp"

// The glass a window's frame showed, as composited: glass whose backdrop has
// not changed is copied back from here when the window repaints over it,
// instead of filtered again. Filtering reads the backdrop a filter's reach
// around the glass, and outside what the compositor repaints that is last
// frame's finished image, the window included; glass is filtered only where
// its backdrop changed, and that reach is repainted first (see preChecks).
struct GlassCache {
    Target target;
    // Framebuffer texels of the cache's top-left; what it holds, in
    // framebuffer texels; what must be filtered again this frame.
    int x = 0, y = 0;
    CRegion valid, refresh;
    // What the cache was composited for.
    Vector2D at{-1e9, -1e9};
    double scale = 0;
    uint64_t revision = ~0ULL;
    const void* monitor = nullptr;
};
static std::map<const void*, GlassCache> caches;
// Caches of destroyed materials, released with the GL context current.
static std::vector<Target> retiredCaches;

static void retireCache(const void* material) {
    auto it = caches.find(material);
    if (it == caches.end())
        return;
    retiredCaches.push_back(it->second.target);
    caches.erase(it);
}

// Forgets a cache that no longer matches where and what the glass is.
static GlassCache& cacheFor(const void* material, Vector2D at, double scale, uint64_t revision, const void* monitor) {
    auto& cache = caches[material];
    if (cache.at != at || cache.scale != scale || cache.revision != revision || cache.monitor != monitor) {
        cache.valid = CRegion{};
        cache.at = at;
        cache.scale = scale;
        cache.revision = revision;
        cache.monitor = monitor;
    }
    return cache;
}

static bool filters(float sigma) { return std::isfinite(sigma) && 3.0F * sigma >= MIN_FILTER_REACH; }

static void upload(GLuint buffer, GLuint binding, const float* floats, size_t count) {
    glBindBuffer(GL_UNIFORM_BUFFER, buffer);
    glBufferSubData(GL_UNIFORM_BUFFER, 0, GLsizeiptr(count * sizeof(float)), floats);
    glBindBufferBase(GL_UNIFORM_BUFFER, binding, buffer);
}

// One axis of the framework's separable Gaussian (material_filter.wgsl).
static void filterPass(const Target& from, const Target& to, int w, int h, float sigma, bool horizontal) {
    glBindFramebuffer(GL_FRAMEBUFFER, to.framebuffer);
    glViewport(0, 0, w, h);
    glUseProgram(gl.filter);
    glActiveTexture(GL_TEXTURE0);
    glBindTexture(GL_TEXTURE_2D, from.texture);
    // `Filter`: direction, sigma, radius; the copied rectangle, inclusive.
    const float block[8] = {horizontal ? 1.F : 0.F, horizontal ? 0.F : 1.F, sigma, std::clamp(std::ceil(3.F * sigma), 0.F, MAX_FILTER_RADIUS), 0, 0, float(w - 1), float(h - 1)};
    upload(gl.filterBlock, 1, block, 8);
    glDrawArrays(GL_TRIANGLES, 0, 3);
}

// How much a filter of `sigma` texels shrinks the copy it runs on: a wide
// blur loses nothing on a halved or quartered copy, and costs a quarter or a
// sixteenth of the texels and a half or a quarter of the taps.
static int filterReduction(float sigma) { return sigma >= 8.F ? 4 : sigma >= 4.F ? 2 : 1; }

// The `cw` by `ch` copied into `gl.source`, filtered by `sigma`: into `full`
// at full resolution, or into `small` at a reduced one. A reduced result is
// exactly `gl.source`'s size divided down, so the material shader, which
// addresses every texture by the source's normalized coordinates, reads both
// alike. Reset `reduced` when writing `gl.source`.
static GLuint filterCopy(Target& full, Target& small, int cw, int ch, float sigma, int& reduced) {
    const int reduction = filterReduction(sigma);
    if (reduction == 1) {
        full.ensure(cw, ch);
        filterPass(gl.source, gl.scratch, cw, ch, sigma, true);
        filterPass(gl.scratch, full, cw, ch, sigma, false);
        return full.texture;
    }
    // Each halving averages 2x2 texels exactly.
    const Target* from = &gl.source;
    int w = cw, h = ch;
    Target* levels[2] = {&gl.half, &gl.quarter};
    for (int level = 0, left = reduction; left > 1; left /= 2, ++level) {
        Target& next = *levels[level];
        next.ensureExact(from->w / 2, from->h / 2);
        const int nw = (w + 1) / 2, nh = (h + 1) / 2;
        if (level >= reduced) {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, from->framebuffer);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, next.framebuffer);
            glBlitFramebuffer(0, 0, nw * 2, nh * 2, 0, 0, nw, nh, GL_COLOR_BUFFER_BIT, GL_LINEAR);
            reduced = level + 1;
        }
        from = &next;
        w = nw;
        h = nh;
    }
    gl.smallScratch.ensureExact(from->w, from->h);
    small.ensureExact(from->w, from->h);
    filterPass(*from, gl.smallScratch, w, h, sigma / float(reduction), true);
    filterPass(gl.smallScratch, small, w, h, sigma / float(reduction), false);
    return small.texture;
}

// Groups `need`'s rectangles so each group is copied and filtered once,
// together with `pad` around it: two groups merge only when copying them
// as one costs no more than apart. A frame's strips stay apart, each with
// its own copy, rather than one copy of the whole window.
struct Cluster {
    CBox box;
    CRegion part;
};
static std::vector<Cluster> clusters(const CRegion& need, double pad) {
    std::vector<Cluster> out;
    for (const auto& rect : need.getRects()) {
        const CBox box{double(rect.x1), double(rect.y1), double(rect.x2 - rect.x1), double(rect.y2 - rect.y1)};
        out.push_back({box, CRegion{box}});
    }
    auto padded = [pad](const CBox& box) { return (box.width + 2 * pad) * (box.height + 2 * pad); };
    for (bool merged = true; merged;) {
        merged = false;
        for (size_t i = 0; i < out.size() && !merged; ++i)
            for (size_t j = i + 1; j < out.size() && !merged; ++j) {
                const double x0 = std::min(out[i].box.x, out[j].box.x), y0 = std::min(out[i].box.y, out[j].box.y);
                const double x1 = std::max(out[i].box.x + out[i].box.width, out[j].box.x + out[j].box.width);
                const double y1 = std::max(out[i].box.y + out[i].box.height, out[j].box.y + out[j].box.height);
                const CBox both{x0, y0, x1 - x0, y1 - y0};
                if (padded(both) <= padded(out[i].box) + padded(out[j].box)) {
                    out[i].box = both;
                    out[i].part.add(out[j].part);
                    out.erase(out.begin() + long(j));
                    merged = true;
                }
            }
    }
    return out;
}

// Issues `draw` once per damaged rectangle: what lies outside the frame's
// damage is not repainted this frame, including the content above the glass.
template <typename F>
static void eachDamaged(const CRegion& damage, F draw) {
    glEnable(GL_SCISSOR_TEST);
    for (const auto& rect : damage.getRects()) {
        glScissor(rect.x1, rect.y1, rect.x2 - rect.x1, rect.y2 - rect.y1);
        draw();
    }
    glDisable(GL_SCISSOR_TEST);
}


static float shortHalf(float w, float h) { return (w <= 0 || h <= 0) ? 0.F : std::min(w, h) / 2.F; }
static float uniformRadius(const Region& r) { return std::clamp(r.radius, 0.F, shortHalf(r.w, r.h)); }
static float bevelWidth(const Region& r) {
    const float half = shortHalf(r.w, r.h);
    return half <= 0 ? 0.F : std::max(0.F, std::min(r.t.bevel_width, half));
}
static float opticalDepth(const Region& r) { return shortHalf(r.w, r.h) <= 0 ? 0.F : std::max(0.F, r.t.optical_depth); }
constexpr float SLOPE_STEP = 2.0F;
constexpr float CENTRE_PULL = 4.0F;
constexpr float CHROMA_PER_DISPERSION = 36.0F;
static float rimHeight(float inside, float rim) {
    if (inside <= 0 || rim <= 0)
        return 0.F;
    return inside >= rim ? rim : std::sqrt(inside * (2.F * rim - inside));
}
static float displacementAt(const Region& r, float inside) {
    const float rim = bevelWidth(r);
    if (rim <= 0 || inside < 0 || r.t.index_of_refraction <= 1)
        return 0.F;
    const float slope = (rimHeight(inside + SLOPE_STEP, rim) - rimHeight(inside - SLOPE_STEP, rim)) / (2.F * SLOPE_STEP);
    return slope * (1.F - 1.F / r.t.index_of_refraction) * r.t.refraction * opticalDepth(r) * 0.75F * (2.F + 0.5F * rimHeight(inside, rim) / rim);
}
static float maxDisplacement(const Region& r) {
    if (shortHalf(r.w, r.h) <= 0)
        return 0.F;
    const float rim = bevelWidth(r);
    const float fringe = CHROMA_PER_DISPERSION * std::max(0.F, r.t.dispersion);
    if (rim <= 0) {
        // Flat glass can refract across its whole face. A zero bevel does not
        // make that sampling footprint vanish: partial hover damage must
        // refresh the displaced background as well as the Gaussian support.
        const float body = 1.5F * std::abs(r.t.refraction) * opticalDepth(r);
        return std::max(body, CENTRE_PULL * std::abs(r.t.refraction)) + fringe;
    }
    if (r.t.index_of_refraction <= 1)
        return fringe;
    float bend = 0;
    for (int step = 0; step <= 64; ++step)
        bend = std::max(bend, displacementAt(r, rim * step / 64.F));
    return bend + CENTRE_PULL * r.t.refraction + fringe;
}
static float blurSigma(const Region& r) { return std::clamp(r.t.blur_sigma, 0.F, MAX_BLUR_SIGMA); }
static std::optional<float> diffusionSigma(const Region& r) {
    if (r.t.color_diffusion > 0 && r.t.color_reach > r.t.blur_sigma)
        return std::clamp(r.t.color_reach, 0.F, MAX_BLUR_SIGMA);
    return std::nullopt;
}
static float samplePadding(const Region& r) {
    const float diffusion = r.t.color_diffusion > 0 ? r.t.color_reach : 0.F;
    return 3.F * std::max(blurSigma(r), std::min(diffusion, MAX_BLUR_SIGMA)) + maxDisplacement(r) + 1.F + 3.F * blurSigma(r);
}


// The probes of one pass on their way back: read into a pixel buffer behind a
// fence, collected without stalling once the GPU is done.
struct Readback {
    GLuint buffer = 0;
    GLsync fence = nullptr;
    wl_resource* resource = nullptr;
    ReadbackPolicy::Stamp stamp;
    ReadbackPolicy::Clock::time_point started;
    std::vector<uint32_t> regions;
};
static std::vector<Readback> readbacks;
static std::vector<GLuint> spareBuffers;
static wl_event_source* pollTimer = nullptr;
static UP<Render::GL::CHyprOpenGLImpl>* openglSlot = nullptr;

static Material* currentMaterial(const Readback& readback) {
    auto it = materials.find(readback.resource);
    return it != materials.end() && it->second->stamp == readback.stamp && !it->second->dirty && !it->second->surface.expired() ? it->second.get() : nullptr;
}

static bool canProbe(Material* material) {
    const auto sameLifetime = std::count_if(readbacks.begin(), readbacks.end(), [material](const auto& readback) {
        return readback.stamp.lifetime == material->stamp.lifetime;
    });
    return ReadbackPolicy::canQueue(readbacks.size(), sameLifetime, material->readbackDisabled);
}

static void sendLuminance(wl_resource* resource, ReadbackPolicy::Stamp expected, uint32_t region, float value) {
    auto it = materials.find(resource);
    if (it == materials.end() || it->second->stamp != expected || it->second->dirty || wl_resource_get_version(resource) < 2)
        return;
    Material* m = it->second.get();
    if (region >= m->sent.size())
        return;
    if (m->sent[region] >= 0 && std::abs(m->sent[region] - value) < 1.F / 128.F)
        return;
    m->sent[region] = value;
    enzui_surface_material_v1_send_luminance(m->resource, region, wl_fixed_from_double(value));
}

// Needs the compositor's GL context; an unfinished fence must not block other clients.
#ifdef ENZUI_MATERIAL_READBACK_TEST
static const char* readbackTestMode() {
    if (const auto* mode = std::getenv("ENZUI_MATERIAL_READBACK_TEST_MODE"))
        return mode;
    static char mode[32];
    mode[0] = '\0';
    if (const auto* root = std::getenv("ENZUI_HYPR_RUNROOT")) {
        const auto path = std::string(root) + "/readback-test-mode";
        if (auto* file = std::fopen(path.c_str(), "r")) {
            if (!std::fgets(mode, sizeof(mode), file))
                mode[0] = '\0';
            std::fclose(file);
        }
    }
    return mode;
}
#endif

static GLenum readbackFenceStatus(const Readback& r) {
#ifdef ENZUI_MATERIAL_READBACK_TEST
    if (std::strcmp(readbackTestMode(), "failed") == 0)
        return GL_WAIT_FAILED;
    if (std::strcmp(readbackTestMode(), "stalled") == 0 ||
        (std::strcmp(readbackTestMode(), "delayed") == 0 && ReadbackPolicy::Clock::now() - r.started < std::chrono::milliseconds(200)))
        return GL_TIMEOUT_EXPIRED;
#endif
    return r.fence ? glClientWaitSync(r.fence, 0, 0) : GL_WAIT_FAILED;
}

static void collectReadbacks() {
    if (readbacks.empty())
        return;
    GLint previous = 0;
    glGetIntegerv(GL_PIXEL_PACK_BUFFER_BINDING, &previous);
    const auto now = ReadbackPolicy::Clock::now();
    ReadbackPolicy::collect(readbacks, [now](const auto& r) {
        const GLenum status = readbackFenceStatus(r);
        const auto fence = status == GL_ALREADY_SIGNALED || status == GL_CONDITION_SATISFIED ? ReadbackPolicy::Fence::Ready
            : status == GL_TIMEOUT_EXPIRED ? ReadbackPolicy::Fence::Pending : ReadbackPolicy::Fence::Failed;
        return ReadbackPolicy::decide(currentMaterial(r) != nullptr, now - r.started, fence);
    }, [](auto& r, auto decision) {
#ifdef ENZUI_MATERIAL_READBACK_TEST
        std::fprintf(stderr, "MATERIAL_READBACK decision=%d lifetime=%llu revision=%llu queue=%zu spare=%zu\n", int(decision),
            static_cast<unsigned long long>(r.stamp.lifetime), static_cast<unsigned long long>(r.stamp.revision), readbacks.size(), spareBuffers.size());
#endif
        bool reusable = decision == ReadbackPolicy::Decision::Deliver;
        if (reusable) {
            glBindBuffer(GL_PIXEL_PACK_BUFFER, r.buffer);
            const auto* texels = static_cast<const uint8_t*>(glMapBufferRange(GL_PIXEL_PACK_BUFFER, 0, GLsizeiptr(r.regions.size() * 4), GL_MAP_READ_BIT));
            if (texels) {
                for (size_t i = 0; i < r.regions.size(); ++i)
                    sendLuminance(r.resource, r.stamp, r.regions[i], texels[i * 4] / 255.F);
                reusable = glUnmapBuffer(GL_PIXEL_PACK_BUFFER) == GL_TRUE;
            } else {
                reusable = false;
            }
        }
        if (decision == ReadbackPolicy::Decision::Failed || (decision == ReadbackPolicy::Decision::Deliver && !reusable)) {
            if (auto* material = currentMaterial(r))
                material->readbackDisabled = true;
        }
        if (r.fence)
            glDeleteSync(r.fence);
        if (reusable && spareBuffers.size() < ReadbackPolicy::MAX_SPARE_BUFFERS)
            spareBuffers.push_back(r.buffer);
        else
            glDeleteBuffers(1, &r.buffer);
    });
    glBindBuffer(GL_PIXEL_PACK_BUFFER, previous);
}

// Readbacks are collected by the next pass, or by this timer when no frame
// comes: a measure-only surface paints nothing that would cause one.
static int pollReadbacks(void*) {
    if (readbacks.empty() || !openglSlot || !*openglSlot)
        return 0;
    (*openglSlot)->makeEGLCurrent();
    collectReadbacks();
    if (!readbacks.empty())
        wl_event_source_timer_update(pollTimer, ReadbackPolicy::POLL_MS);
    return 0;
}

static void dropReadbacks() {
    if (openglSlot && *openglSlot)
        (*openglSlot)->makeEGLCurrent();
    for (auto& r : readbacks) {
        if (r.fence)
            glDeleteSync(r.fence);
        glDeleteBuffers(1, &r.buffer);
    }
    readbacks.clear();
    if (!spareBuffers.empty())
        glDeleteBuffers(GLsizei(spareBuffers.size()), spareBuffers.data());
    spareBuffers.clear();
}


class GlassPass final : public IPassElement {
    std::vector<Region> regions;
    Shadow shadow;
    // The surface's top-left on its monitor, logical; and the surface-to-box scale.
    Vector2D origin;
    Vector2D stretch;
    float alpha;
    // The material whose regions this measures, if its client reads them;
    // and whether it only measures.
    wl_resource* materialResource;
    ReadbackPolicy::Stamp materialStamp;
    bool measureOnly;
    // The cutouts over the glass, surface-local logical.
    std::vector<Cutout> cutouts;
    // The material whose glass this composites, for its cache.
    Material* owner;

  public:
    GlassPass(std::vector<Region> r, Shadow sh, Vector2D o, Vector2D s, float a, Material* m, bool only, std::vector<Cutout> v, Material* own) :
        regions(std::move(r)), shadow(sh), origin(o), stretch(s), alpha(a), materialResource(m ? m->resource : nullptr),
        materialStamp(m ? m->stamp : ReadbackPolicy::Stamp{}), measureOnly(only), cutouts(std::move(v)), owner(own) {}
    // The soft shadow outside region `r` (framebuffer `x, y, w, h`), within
    // `damage`; the framebuffer is bound, blending on.
    void drawShadow(const Region& r, double x, double y, double w, double h, int fbw, int fbh, float scale, float reach, const CRegion& damage) {
        const float halfW = w / 2, halfH = h / 2;
        glUseProgram(gl.shadow);
        glUniform4f(glGetUniformLocation(gl.shadow, "surface"), fbw, fbh, scale, 0);
        glUniform4f(glGetUniformLocation(gl.shadow, "region"), x + halfW, y + halfH, halfW, halfH);
        glUniform1f(glGetUniformLocation(gl.shadow, "reach"), reach);
        glUniform4f(glGetUniformLocation(gl.shadow, "shape"), uniformRadius(r) * stretch.x * scale, 0, 0, 0);
        glUniform4f(glGetUniformLocation(gl.shadow, "shadow"), shadow.blur * scale, shadow.offset * scale, shadow.opacity * alpha, 0);
        eachDamaged(damage, [] { glDrawArrays(GL_TRIANGLES, 0, 6); });
    }
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    bool disableSimplification() override { return true; }
    bool requiresFullDamage() override { return false; }
    const char* passName() override { return "EnzMaterial"; }
    ePassElementType type() override { return EK_CUSTOM; }
    std::vector<UP<IPassElement>> draw() override {
        auto& data = renderer().m_renderData;
        const auto monitor = data.pMonitor.lock();
        const auto fb = data.currentFB ? dynamic_cast<Render::GL::CGLFramebuffer*>(data.currentFB.get()) : nullptr;
        // Glass is filtered from the live backdrop; a picture of a window
        // drawn aside (a snapshot) has none, so its glass is left out.
        if (!monitor || !fb || alpha <= 0.F || !gl.ready() || data.renderingTransformedSource)
            return {};
        collectReadbacks();
        const float scale = monitor->m_scale;
        const int fbw = int(fb->m_size.x), fbh = int(fb->m_size.y);
        if (fbw <= 0 || fbh <= 0)
            return {};

        Vector2D at = origin;
        Vector2D stretch = this->stretch;
        CRegion damage = data.damage.copy();
        if (data.renderModif.enabled)
            for (const auto& [type, value] : data.renderModif.modifs) {
                if (type == Render::SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE) {
                    const auto shift = std::any_cast<Vector2D>(value);
                    at += shift / scale;
                    damage.translate(shift);
                } else if (type == Render::SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE) {
                    const auto factor = std::any_cast<float>(value);
                    if (!std::isfinite(factor) || factor <= 0)
                        return {};
                    at = at * factor;
                    stretch = stretch * factor;
                    damage.scale(factor);
                } else
                    return {};
            }

        // Hyprland caches bindings, viewport, blend and caps; everything this
        // touches is restored to exactly what it was, so its caches stay true.
        GLint previousVao = 0, previousProgram = 0, previousViewport[4] = {}, previousActive = 0, drawFb = 0, readFb = 0;
        GLint blendSrc = 0, blendDst = 0, blendSrcA = 0, blendDstA = 0, previousTextures[3] = {};
        glGetIntegerv(GL_VERTEX_ARRAY_BINDING, &previousVao);
        glGetIntegerv(GL_CURRENT_PROGRAM, &previousProgram);
        glGetIntegerv(GL_VIEWPORT, previousViewport);
        glGetIntegerv(GL_ACTIVE_TEXTURE, &previousActive);
        glGetIntegerv(GL_DRAW_FRAMEBUFFER_BINDING, &drawFb);
        glGetIntegerv(GL_READ_FRAMEBUFFER_BINDING, &readFb);
        GLint uniformBuffer = 0, packBuffer = 0;
        glGetIntegerv(GL_UNIFORM_BUFFER_BINDING, &uniformBuffer);
        glGetIntegerv(GL_PIXEL_PACK_BUFFER_BINDING, &packBuffer);
        glGetIntegerv(GL_BLEND_SRC_RGB, &blendSrc);
        glGetIntegerv(GL_BLEND_DST_RGB, &blendDst);
        glGetIntegerv(GL_BLEND_SRC_ALPHA, &blendSrcA);
        glGetIntegerv(GL_BLEND_DST_ALPHA, &blendDstA);
        for (int i = 0; i < 3; ++i) {
            glActiveTexture(GL_TEXTURE0 + i);
            glGetIntegerv(GL_TEXTURE_BINDING_2D, &previousTextures[i]);
        }
        const bool blend = glIsEnabled(GL_BLEND), scissor = glIsEnabled(GL_SCISSOR_TEST), stencil = glIsEnabled(GL_STENCIL_TEST);
        glDisable(GL_SCISSOR_TEST);
        glDisable(GL_STENCIL_TEST);
        glBindVertexArray(gl.vao);
        // The visible glass in framebuffer texels, a texel wider for rounding.
        const CRegion shown = visibleGlass(regions, cutouts, at, stretch, scale);
        std::vector<uint32_t> probed;
        const Readback identity{.resource = materialResource, .stamp = materialStamp};
        auto* measured = currentMaterial(identity);
        const bool probing = measured && measured->wantsReadings && measured->unmeasured && canProbe(measured);
        for (auto& retired : retiredCaches)
            retired.release();
        retiredCaches.clear();
        // The cache covers the regions' box; a fading window composites
        // aside and is not kept.
        GlassCache* cache = nullptr;
        if (owner && alpha >= 0.999F && !measureOnly && !regions.empty()) {
            cache = &cacheFor(owner, at, scale, owner->stamp.revision, monitor.get());
            double x0 = 1e9, y0 = 1e9, x1 = -1e9, y1 = -1e9;
            for (const auto& r : regions) {
                x0 = std::min(x0, (at.x + r.x * stretch.x) * scale);
                y0 = std::min(y0, (at.y + r.y * stretch.y) * scale);
                x1 = std::max(x1, (at.x + (r.x + r.w) * stretch.x) * scale);
                y1 = std::max(y1, (at.y + (r.y + r.h) * stretch.y) * scale);
            }
            const auto raster = glassRasterBox({x0, y0, x1 - x0, y1 - y0});
            const int cx = int(raster.x), cy = int(raster.y);
            const int cw = int(raster.w), ch = int(raster.h);
            if (cw <= 0 || ch <= 0)
                cache = nullptr;
            else if (!cache->target.texture || cache->target.w != cw || cache->target.h != ch || cache->x != cx || cache->y != cy) {
                cache->target.ensureExact(cw, ch);
                cache->x = cx;
                cache->y = cy;
                cache->valid = CRegion{};
            }
        }
        auto blitRects = [&](const CRegion& rects, GLuint from, GLuint to, int dx, int dy) {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, from);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, to);
            for (const auto& b : rects.getRects())
                glBlitFramebuffer(b.x1, b.y1, b.x2, b.y2, b.x1 + dx, b.y1 + dy, b.x2 + dx, b.y2 + dy, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        };

        // The copy a group's filters read: the group and their reach around
        // it, within the region's reach and the framebuffer.
        auto copyRect = [&](const Cluster& group, const CBox& box, float pad) {
            return std::array<int, 4>{
                std::clamp(int(std::floor(std::max(group.box.x, box.x) - pad)), 0, fbw),
                std::clamp(int(std::floor(std::max(group.box.y, box.y) - pad)), 0, fbh),
                std::clamp(int(std::ceil(std::min(group.box.x + group.box.width, box.x + box.width) + pad)), 0, fbw),
                std::clamp(int(std::ceil(std::min(group.box.y + group.box.height, box.y + box.height) + pad)), 0, fbh),
            };
        };
        // Every region is planned before anything is drawn: what it
        // composites (`need`), what of that comes back from the cache
        // (`restore`), and the groups filtered for the rest. A measure-only
        // region keeps its whole copy: the probe reads all of it. Otherwise
        // only the visible glass this frame repaints is composited, and the
        // copies and filters cover just that and the reach of the filters
        // around it.
        struct Plan {
            CRegion need, restore;
            std::vector<Cluster> groups;
        };
        std::vector<Plan> plans(regions.size());
        CRegion reads;
        size_t copies = 0;
        bool restores = false;
        for (uint32_t index = 0; index < regions.size(); ++index) {
            const auto& r = regions[index];
            if (r.w <= 0 || r.h <= 0)
                continue;
            const CBox box{(at.x + r.x * stretch.x) * scale, (at.y + r.y * stretch.y) * scale, r.w * stretch.x * scale, r.h * stretch.y * scale};
            const float pad = samplePadding(r) * scale;
            auto& plan = plans[index];
            plan.need = measureOnly ? CRegion{box} : shown.copy().intersect(glassRasterBox(box));
            plan.need.intersect(damage);
            // Glass whose backdrop has not changed is copied back as it was
            // composited; only the rest is filtered.
            if (cache && !measureOnly && !plan.need.empty()) {
                plan.restore = plan.need.copy().intersect(cache->valid).subtract(cache->refresh);
                plan.need.subtract(plan.restore);
                restores |= !plan.restore.empty();
            }
            // The probe reads the region only where this frame repaints it.
            const bool probe = probing && !CRegion{box}.intersect(damage).empty();
            if (plan.need.empty() && !probe)
                continue;
            plan.groups = measureOnly ? std::vector<Cluster>{{box, plan.need}} : clusters(plan.need, pad);
            // A probe reads the whole region, so it gets a copy of its own
            // that is never composited: past this frame's damage that copy
            // holds last frame's image, the window included.
            if (probe && !measureOnly)
                plan.groups.insert(plan.groups.begin(), Cluster{box, CRegion{}});
            for (const auto& group : plan.groups) {
                const auto [cx0, cy0, cx1, cy1] = copyRect(group, box, pad);
                if (cx1 > cx0 && cy1 > cy0) {
                    reads.add(CBox{double(cx0), double(cy0), double(cx1 - cx0), double(cy1 - cy0)});
                    ++copies;
                }
            }
        }
        // Filters read the backdrop as it was before this pass drew any
        // glass. A copy taken after another group's glass, cached glass or a
        // shadow landed beside it would filter that in, and since groups
        // follow the damage, the same glass would look different from one
        // frame to the next. One copy that nothing was drawn before reads
        // the framebuffer itself.
        const bool snapshot = copies > 1 || (copies == 1 && (restores || regions.size() > 1));
        int readX = 0, readY = 0;
        if (snapshot) {
            const auto extents = reads.getExtents();
            readX = int(extents.x);
            readY = int(extents.y);
            gl.backdrop.ensure(int(extents.width), int(extents.height));
            blitRects(reads, fb->getFBID(), gl.backdrop.framebuffer, -readX, -readY);
        }

        for (uint32_t index = 0; index < regions.size(); ++index) {
            const auto& r = regions[index];
            if (r.w <= 0 || r.h <= 0)
                continue;
            // The offload framebuffer is upright with its top row first, before the output
            // transform.
            const double x = (at.x + r.x * stretch.x) * scale, w = r.w * stretch.x * scale;
            const double y = (at.y + r.y * stretch.y) * scale, h = r.h * stretch.y * scale;
            const float pad = samplePadding(r) * scale;
            const CBox box{x, y, w, h};
            const bool whole = measureOnly;
            const auto& [need, restore, groups] = plans[index];
            if (!restore.empty())
                blitRects(restore.copy().translate({-double(cache->x), -double(cache->y)}), cache->target.framebuffer, fb->getFBID(), cache->x, cache->y);
            const bool shadowed = shadow.opacity > 0 && shadow.blur > 0 && !measureOnly;
            const float reach = (shadow.blur * 1.5F + std::abs(shadow.offset)) * scale;
            const bool shadowDamaged = shadowed && !damage.copy().intersect(CBox{x - reach, y - reach, w + 2 * reach, h + 2 * reach}).empty();
            if (groups.empty() && !shadowDamaged)
                continue;
            // Only the shadow around the glass is repainted.
            if (groups.empty()) {
                glBindFramebuffer(GL_FRAMEBUFFER, fb->getFBID());
                glViewport(0, 0, fbw, fbh);
                glEnable(GL_BLEND);
                glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
                drawShadow(r, x, y, w, h, fbw, fbh, scale, reach, damage);
                continue;
            }
            const float sigma = blurSigma(r) * scale;
            // Matches iced_core::material::Uniform in physical pixels; the
            // copy's origin is the cluster's.
            const float sx = stretch.x * scale;
            auto block = [&](float originX, float originY, float surfaceW, float surfaceH, int cx0, int cy0) {
                return std::array<float, 32>{
                    surfaceW, surfaceH, scale, 0,
                    float(x + w / 2 - originX), float(y + h / 2 - originY), float(w / 2), float(h / 2),
                    uniformRadius(r) * sx, bevelWidth(r) * sx, opticalDepth(r) * sx, r.t.profile_curvature,
                    r.t.refraction, r.t.index_of_refraction, r.t.dispersion, sigma,
                    r.t.tint_opacity, r.t.saturation, r.t.color_diffusion, r.t.color_reach * scale,
                    r.t.highlight, r.t.brightness, r.t.light_angle * float(M_PI) / 180.F, r.t.contrast_protection,
                    r.tint[0], r.tint[1], r.tint[2], r.tint[3],
                    r.t.protected_inset * scale, float(cx0) - originX, float(cy0) - originY, r.t.tint_opacity >= 1.F ? 1.F : 0.F,
                };
            };
            for (const auto& group : groups) {
                const auto [cx0, cy0, cx1, cy1] = copyRect(group, box, pad);
                const int cw = cx1 - cx0, ch = cy1 - cy0;
                if (cw <= 0 || ch <= 0)
                    continue;

                gl.source.ensure(cw, ch);
                gl.scratch.ensure(cw, ch);
                glBindFramebuffer(GL_READ_FRAMEBUFFER, snapshot ? gl.backdrop.framebuffer : fb->getFBID());
                glBindFramebuffer(GL_DRAW_FRAMEBUFFER, gl.source.framebuffer);
                if (snapshot)
                    glBlitFramebuffer(cx0 - readX, cy0 - readY, cx1 - readX, cy1 - readY, 0, 0, cw, ch, GL_COLOR_BUFFER_BIT, GL_NEAREST);
                else
                    glBlitFramebuffer(cx0, cy0, cx1, cy1, 0, 0, cw, ch, GL_COLOR_BUFFER_BIT, GL_NEAREST);
                glDisable(GL_BLEND);

                int reduced = 0;
                GLuint blurredTexture = gl.source.texture;
                if (filters(sigma))
                    blurredTexture = filterCopy(gl.blurred, gl.smallBlurred, cw, ch, sigma, reduced);
                GLuint diffusedTexture = blurredTexture;
                if (const auto spill = diffusionSigma(r); spill && filters(*spill * scale))
                    diffusedTexture = filterCopy(gl.diffused, gl.smallDiffused, cw, ch, *spill * scale, reduced);

                const auto onFrame = block(0, 0, fbw, fbh, cx0, cy0);
                upload(gl.materialBlock, 4, onFrame.data(), onFrame.size());
                const GLuint textures[3] = {gl.source.texture, blurredTexture, diffusedTexture};
                for (int i = 0; i < 3; ++i) {
                    glActiveTexture(GL_TEXTURE0 + i);
                    glBindTexture(GL_TEXTURE_2D, textures[i]);
                }

                // Probe only freshly repainted pixels; other pixels may still
                // contain last frame's glass. A probed region is one group.
                if (probing && (whole || group.part.empty())) {
                    gl.probes.ensure(int(MAX_REGIONS), 1);
                    glBindFramebuffer(GL_FRAMEBUFFER, gl.probes.framebuffer);
                    glViewport(GLint(probed.size()), 0, 1, 1);
                    glUseProgram(gl.probe);
                    glDrawArrays(GL_TRIANGLES, 0, 3);
                    probed.push_back(index);
                }
                if (measureOnly || group.part.empty())
                    continue;

                glBindFramebuffer(GL_FRAMEBUFFER, fb->getFBID());
                glViewport(0, 0, fbw, fbh);
                glEnable(GL_BLEND);
                glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
                if (alpha >= 0.999F) {
                    glUseProgram(gl.material);
                    eachDamaged(group.part, [] { glDrawArrays(GL_TRIANGLES, 0, 6); });
                } else {
                    // Composited aside over the copied rectangle, then laid
                    // over the frame at the window's opacity.
                    gl.faded.ensure(cw, ch);
                    glBindFramebuffer(GL_FRAMEBUFFER, gl.faded.framebuffer);
                    glViewport(0, 0, cw, ch);
                    GLfloat clear[4] = {};
                    glGetFloatv(GL_COLOR_CLEAR_VALUE, clear);
                    glClearColor(0, 0, 0, 0);
                    glClear(GL_COLOR_BUFFER_BIT);
                    glClearColor(clear[0], clear[1], clear[2], clear[3]);
                    glDisable(GL_BLEND);
                    const auto aside = block(float(cx0), float(cy0), float(cw), float(ch), cx0, cy0);
                    upload(gl.materialBlock, 4, aside.data(), aside.size());
                    glUseProgram(gl.material);
                    glDrawArrays(GL_TRIANGLES, 0, 6);
                    glBindFramebuffer(GL_FRAMEBUFFER, fb->getFBID());
                    glViewport(cx0, cy0, cw, ch);
                    glEnable(GL_BLEND);
                    glUseProgram(gl.fade);
                    glActiveTexture(GL_TEXTURE0);
                    glBindTexture(GL_TEXTURE_2D, gl.faded.texture);
                    glUniform1i(glGetUniformLocation(gl.fade, "fade_texture"), 0);
                    glUniform2f(glGetUniformLocation(gl.fade, "origin"), float(cx0), float(cy0));
                    glUniform1f(glGetUniformLocation(gl.fade, "opacity"), alpha);
                    eachDamaged(group.part, [] { glDrawArrays(GL_TRIANGLES, 0, 3); });
                    glViewport(0, 0, fbw, fbh);
                }
            }
            // What was filtered is kept, before the shadow lands beside it.
            if (cache && !whole && !need.empty()) {
                blitRects(need, fb->getFBID(), cache->target.framebuffer, -cache->x, -cache->y);
                cache->valid.add(need);
            }
            // The shadow lands after every copy, so the glass never samples
            // it; it lies outside the glass, so it never covers it either.
            if (shadowDamaged && !measureOnly) {
                glBindFramebuffer(GL_FRAMEBUFFER, fb->getFBID());
                glViewport(0, 0, fbw, fbh);
                glEnable(GL_BLEND);
                glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
                drawShadow(r, x, y, w, h, fbw, fbh, scale, reach, damage);
            }
        }
        if (!probed.empty()) {
            GLuint buffer = 0;
            if (spareBuffers.empty()) {
                glGenBuffers(1, &buffer);
                glBindBuffer(GL_PIXEL_PACK_BUFFER, buffer);
                glBufferData(GL_PIXEL_PACK_BUFFER, GLsizeiptr(MAX_REGIONS * 4), nullptr, GL_STREAM_READ);
            } else {
                buffer = spareBuffers.back();
                spareBuffers.pop_back();
                glBindBuffer(GL_PIXEL_PACK_BUFFER, buffer);
            }
            glBindFramebuffer(GL_READ_FRAMEBUFFER, gl.probes.framebuffer);
            glReadPixels(0, 0, GLsizei(probed.size()), 1, GL_RGBA, GL_UNSIGNED_BYTE, nullptr);
            glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
            auto fence = glFenceSync(GL_SYNC_GPU_COMMANDS_COMPLETE, 0);
#ifdef ENZUI_MATERIAL_READBACK_TEST
            if (std::strcmp(readbackTestMode(), "null") == 0 && fence) {
                glDeleteSync(fence);
                fence = nullptr;
            }
#endif
            if (fence) {
                readbacks.push_back({buffer, fence, measured->resource, measured->stamp, ReadbackPolicy::Clock::now(), std::move(probed)});
                measured->unmeasured = false;
                measured->measuredAt = origin;
                if (pollTimer)
                    wl_event_source_timer_update(pollTimer, ReadbackPolicy::POLL_MS);
            } else {
                measured->readbackDisabled = true;
                glDeleteBuffers(1, &buffer);
#ifdef ENZUI_MATERIAL_READBACK_TEST
                std::fprintf(stderr, "MATERIAL_READBACK null-fence disabled lifetime=%llu\n", static_cast<unsigned long long>(measured->stamp.lifetime));
#endif
            }
        }
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, drawFb);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, readFb);
        glBindBuffer(GL_UNIFORM_BUFFER, uniformBuffer);
        glBindBuffer(GL_PIXEL_PACK_BUFFER, packBuffer);
        for (int i = 0; i < 3; ++i) {
            glActiveTexture(GL_TEXTURE0 + i);
            glBindTexture(GL_TEXTURE_2D, previousTextures[i]);
        }
        glActiveTexture(previousActive);
        glBindVertexArray(previousVao);
        glUseProgram(previousProgram);
        glViewport(previousViewport[0], previousViewport[1], previousViewport[2], previousViewport[3]);
        glBlendFuncSeparate(blendSrc, blendDst, blendSrcA, blendDstA);
        blend ? glEnable(GL_BLEND) : glDisable(GL_BLEND);
        scissor ? glEnable(GL_SCISSOR_TEST) : glDisable(GL_SCISSOR_TEST);
        stencil ? glEnable(GL_STENCIL_TEST) : glDisable(GL_STENCIL_TEST);
        return {};
    }
};

// Where window `w`'s surface is drawn on `monitor`: its top-left, monitor
// logical, as the renderer places it this frame (animated offsets included),
// and the surface-to-box scale.
static std::pair<Vector2D, Vector2D> glassPlacement(PHLWINDOW w, PHLMONITOR monitor) {
    const auto workspace = w->m_workspace;
    auto pos = w->position(Desktop::View::IGeometric::GEOMETRIC_CURRENT) + w->presentation().floatingOffset();
    if (workspace && !(w->m_state & Desktop::View::eWindowState::WINDOW_STATE_PINNED))
        pos += workspace->m_renderOffset->value();
    const auto size = w->size(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
    const auto geometry = w->backend().geometry().box;
    // Hyprland crops ordinary surface buffers to the window box; only small surfaces are
    // scaled.
    Vector2D stretch = {1, 1};
    if (const auto view = w->wlSurface(); view && view->small() && !view->m_fillIgnoreSmall) {
        const auto reported = w->backend().reportedSize();
        if (reported.x > 0 && reported.y > 0)
            stretch = {size.x / reported.x, size.y / reported.y};
    }
    return {pos - monitor->m_position - Vector2D{geometry.x, geometry.y} * stretch, stretch};
}

bool hasGlass(PHLWINDOW w) {
    const auto surface = w && w->wlSurface() ? w->wlSurface()->resource() : nullptr;
    const auto* m = surface ? forSurface(surface.get()) : nullptr;
    return m && !m->current.empty() && !m->currentMeasureOnly;
}

static void addForWindow(PHLWINDOW w) {
    if (!w || !w->wlSurface())
        return;
    auto surface = w->wlSurface()->resource();
    auto* m = surface ? forSurface(surface.get()) : nullptr;
    if (!m)
        return;
    if (m->current.empty())
        return;
    const auto monitor = renderer().m_renderData.pMonitor.lock();
    if (!monitor)
        return;
    const auto [origin, stretch] = glassPlacement(w, monitor);
    const auto geometry = w->backend().geometry().box;
    if (origin != m->measuredAt)
        m->unmeasured = true;
    // Hyprland crops the surface to the window's box, so the glass is cropped
    // there too: while a resize outruns the client, its regions describe the
    // last size it drew, and would otherwise show past the window, square.
    const auto size = w->size(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
    const float shownX = geometry.x, shownY = geometry.y;
    const float shownW = stretch.x > 0 ? size.x / stretch.x : 0.F, shownH = stretch.y > 0 ? size.y / stretch.y : 0.F;
    auto regions = m->current;
    for (auto& r : regions) {
        const float x0 = std::max(r.x, shownX), y0 = std::max(r.y, shownY);
        const float x1 = std::min(r.x + r.w, shownX + shownW), y1 = std::min(r.y + r.h, shownY + shownH);
        r.x = x0;
        r.y = y0;
        r.w = std::max(0.F, x1 - x0);
        r.h = std::max(0.F, y1 - y0);
        r.radius = std::min(r.radius, std::min(r.w, r.h) / 2);
    }
    // Hyprland rounds the window's corners after the glass is drawn beneath
    // it: glass reaching a corner of the window keeps inside that rounding,
    // so none shows past the border. A region has one radius, so one that
    // reaches any corner is rounded: while a window grows ahead of its
    // client, the glass reaches only the corners it shares with the window.
    const float rounding = w->presentation().rounding();
    if (rounding > 0)
        for (auto& r : regions) {
            constexpr float NEAR = 1.F;
            const bool left = r.x <= shownX + NEAR, top = r.y <= shownY + NEAR;
            const bool right = r.x + r.w >= shownX + shownW - NEAR, bottom = r.y + r.h >= shownY + shownH - NEAR;
            if ((left || right) && (top || bottom))
                r.radius = std::max(r.radius, std::min(rounding, std::min(r.w, r.h) / 2));
        }
    renderer().addPassElement(makeUnique<GlassPass>(std::move(regions), m->currentShadow, origin, stretch, w->presentation().alphaTotal(),
                                                    wl_resource_get_version(m->resource) >= 2 ? m : nullptr, m->currentMeasureOnly,
                                                    m->currentCutouts, m));
}

void init(HANDLE handle) {
    owner = handle;
    compositorSlot = static_cast<UP<CCompositor>*>(dlsym(RTLD_DEFAULT, "g_pCompositor"));
    rendererSlot = static_cast<UP<Render::IHyprRenderer>*>(dlsym(RTLD_DEFAULT, "g_pHyprRenderer"));
    openglSlot = static_cast<UP<Render::GL::CHyprOpenGLImpl>*>(dlsym(RTLD_DEFAULT, "_ZN6Render2GL13g_pHyprOpenGLE"));
    if (!compositorSlot || !rendererSlot || !*compositorSlot || !*rendererSlot)
        throw std::runtime_error("enzui-material: compositor globals are not reachable");
    // A software rasterizer would spend whole frames on the filters, so the
    // glass is not offered there and clients keep their solid panels.
    const auto* rasterizer = reinterpret_cast<const char*>(glGetString(GL_RENDERER));
    const std::string_view name = rasterizer ? rasterizer : "";
    if (name.find("llvmpipe") != std::string_view::npos || name.find("softpipe") != std::string_view::npos || name.find("SwiftShader") != std::string_view::npos) {
        LOG(Log::DEBUG, "enzui-material: software renderer {}, no liquid glass", name);
    } else {
        global = wl_global_create(compositor().m_wlDisplay, &enzui_material_manager_v1_interface, 3, nullptr, bindManager);
        if (!global)
            throw std::runtime_error("enzui-material: cannot create the global");
    }
    // A change in the backdrop within a filter's reach of visible glass
    // repaints that glass whole, so it is never recomposited from a partly
    // stale backdrop. What the window drew itself is never its backdrop: its
    // own damage alone repaints only what it touches, and glass under its
    // opaque content (its cutouts) is neither filtered nor painted.
    pollTimer = wl_event_loop_add_timer(compositor().m_wlEventLoop, pollReadbacks, nullptr);
    checksListener = Event::bus()->m_events.render.preChecks.listen([](PHLMONITOR monitor) {
        if (!monitor)
            return;
        const auto pending = monitor->m_damage.getBufferDamage(1);
        for (auto& [_, m] : materials) {
            CRegion own = std::move(m->ownDamage);
            m->ownDamage = CRegion{};
            if (pending.empty() || m->current.empty() || m->currentMeasureOnly)
                continue;
            const auto surface = m->surface.lock();
            const auto view = surface ? Desktop::View::CWLSurface::fromResource(surface) : nullptr;
            const auto window = view ? Desktop::View::CWindow::fromView(view->view()) : nullptr;
            if (!window)
                continue;
            float pad = 0;
            for (const auto& r : m->current)
                pad = std::max(pad, samplePadding(r));
            const auto [at, stretch] = glassPlacement(window, monitor);
            const double scale = monitor->m_scale;
            const CRegion glass = visibleGlass(m->current, m->currentCutouts, at, stretch, scale);
            if (glass.empty())
                continue;
            // To the output's buffer texels.
            CRegion foreign = pending.copy();
            if (!own.empty())
                foreign.subtract(own.copy().translate(-monitor->m_position).scale(scale).expand(1));
            const double reach = std::ceil(pad * scale) + 2;
            for (const auto& r : m->current) {
                const CBox box = CBox{(at.x + r.x * stretch.x) * scale, (at.y + r.y * stretch.y) * scale, r.w * stretch.x * scale, r.h * stretch.y * scale};
                if (!foreign.copy().intersect(box.copy().expand(reach)).empty())
                    m->unmeasured = true;
            }
            // The glass filtered again this frame: within a filter's reach of
            // a change behind it, or repainted without a kept composite. Its
            // reach is repainted too, so the filters read this frame's
            // backdrop there, not last frame's image with the window on it.
            auto& cache = cacheFor(m.get(), at, scale, m->stamp.revision, monitor.get());
            CRegion refresh = foreign.copy().expand(reach).add(pending.copy().subtract(cache.valid)).intersect(glass);
            cache.valid.subtract(refresh);
            cache.refresh = refresh;
            if (!refresh.empty())
                monitor->m_damage.damage(refresh.copy().expand(reach));
        }
    });
    AppearanceTransition::init(handle);
    stageListener = Event::bus()->m_events.render.stage.listen([](eRenderStage stage) {
        if (stage == RENDER_PRE)
            glazed.clear();
        if (stage != RENDER_PRE_WINDOW)
            return;
        const auto window = renderer().m_renderData.currentWindow.lock();
        if (!window || std::ranges::find(glazed, window.get()) != glazed.end())
            return;
        glazed.push_back(window.get());
        addForWindow(window);
    });
}

void exit() {
    AppearanceTransition::exit(owner);
    stageListener.reset();
    checksListener.reset();
    glazed.clear();
    std::vector<wl_resource*> resources;
    for (auto& [resource, _] : materials)
        resources.push_back(resource);
    for (auto* resource : resources)
        wl_resource_destroy(resource);
    materials.clear();
    for (auto* manager : std::vector<wl_resource*>(managers))
        wl_resource_destroy(manager);
    managers.clear();
    if (global)
        wl_global_destroy(global);
    global = nullptr;
    if (pollTimer)
        wl_event_source_remove(pollTimer);
    pollTimer = nullptr;
    dropReadbacks();
    for (auto& [_, cache] : caches)
        cache.target.release();
    caches.clear();
    for (auto& retired : retiredCaches)
        retired.release();
    retiredCaches.clear();
    gl.release();
}

} // namespace EnzMaterial
