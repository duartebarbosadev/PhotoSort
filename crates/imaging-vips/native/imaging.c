/* Native ownership and error boundary only. Codecs, resampling and ICC math
 * belong to libvips and Little CMS. This adapter never mutates source files. */
#include <lcms2.h>
#include <vips/vips.h>
#include <stdint.h>
#include <string.h>

/* Default memory for a render when the caller names none. */
#define MAX_BYTES ((uint64_t)512 * 1024 * 1024)
#define MAX_PROFILE ((size_t)4 * 1024 * 1024)
/* No real image is larger: a header claiming more is corrupt or a
 * decompression bomb. Memory, not these, bounds real photos. */
#define MAX_EDGE (1 << 20)
#define MAX_PIXELS ((uint64_t)1 << 32)
/* Previews run unattended (grid, thumbnails) while holding the render lock:
 * past a gigapixel a hostile header would stream for minutes. */
#define MAX_PREVIEW_PIXELS ((uint64_t)1 << 30)

typedef int (*Cancel)(void *);
typedef struct {
    void *pixels;
    size_t length;
    uint32_t width, height, source_width, source_height, sample_bytes, profiled, source_sample_bytes;
    /* Set when a full-size render was refused for memory: what it needs. */
    uint64_t needed;
} Output;
typedef struct { Cancel check; void *context; } Progress;

typedef struct { char *text; size_t capacity; } Warnings;
static GPrivate current_warnings = G_PRIVATE_INIT(NULL);
static void warning_log(const gchar *domain, GLogLevelFlags level,
    const gchar *message, gpointer unused) {
    (void)unused;
    Warnings *warnings = g_private_get(&current_warnings);
    if (warnings) {
        if (!warnings->text[0]) g_strlcpy(warnings->text, message, warnings->capacity);
    } else g_log_default_handler(domain, level, message, NULL);
}

static void progress(VipsImage *image, VipsProgress *ignored, void *context) {
    (void)ignored;
    Progress *p = context;
    if (p->check(p->context)) vips_image_set_kill(image, TRUE);
}
static int fail(char *error, size_t capacity, const char *message) {
    g_strlcpy(error, message, capacity);
    return -1;
}
static int native_failure(char *error, size_t capacity) {
    char *message = vips_error_buffer_copy();
    int result = fail(error, capacity, message && message[0] ? message : "Native image preparation failed");
    g_free(message);
    return result;
}
int photosort_vips_init(char *error, size_t capacity) {
    if (vips_init("photosort")) return native_failure(error, capacity);
    if (!vips_icc_present()) return fail(error, capacity, "libvips requires Little CMS support");
    /* One Rust image actor owns reuse. No hidden global operation/file cache.
     * Set process-wide policy once; never change it while jobs are running. */
    g_log_set_handler("VIPS", G_LOG_LEVEL_WARNING, warning_log, NULL);
    vips_cache_set_max(0);
    vips_cache_set_max_mem(0);
    vips_cache_set_max_files(0);
    vips_concurrency_set(4);
    return 0;
}
void photosort_vips_free(void *pixels) { g_free(pixels); }

int photosort_vips_render(const char *path, uint32_t edge,
    uint32_t x, uint32_t y, uint32_t width, uint32_t height,
    uint32_t orientation, uint32_t tile_edge, uint64_t max_bytes,
    Cancel cancelled, void *context, Output *output, char *error, size_t capacity,
    char *warning, size_t warning_capacity) {
    VipsSource *source = NULL;
    VipsImage *header = NULL, *working = NULL, *colored = NULL, *sized = NULL;
    VipsImage *cast = NULL, *rgba = NULL;
    const void *profile = NULL;
    size_t profile_length = 0;
    cmsHPROFILE icc = NULL;
    int result = -1;
    gulong signal = 0;
    Progress p = {cancelled, context};
    Warnings warnings = {warning, warning_capacity};
    void *previous_warnings = g_private_get(&current_warnings);
    warning[0] = '\0';
    g_private_set(&current_warnings, &warnings);
    memset(output, 0, sizeof(*output));
    vips_error_clear();
    if (cancelled(context)) { fail(error, capacity, "Image preparation cancelled"); goto done; }
    /* A VipsSource treats bracket characters as literal filename bytes. */
    source = vips_source_new_from_file(path);
    if (!source) goto native_error;
    header = vips_image_new_from_source(source, "", "access", VIPS_ACCESS_SEQUENTIAL,
        "fail_on", VIPS_FAIL_ON_WARNING, NULL);
    if (!header) goto native_error;
    int w = vips_image_get_width(header), h = vips_image_get_height(header);
    int bands = vips_image_get_bands(header);
    size_t sample = vips_format_sizeof(vips_image_get_format(header));
    if (!max_bytes) max_bytes = MAX_BYTES;
    /* A downsized preview never holds the source raster: loaders shrink on load
     * or stream it. A region decodes only its crop. Both are bounded by their
     * output below; only a full-size render holds the whole source. */
    const int preview = !width && !height && edge && ((uint32_t)w > edge || (uint32_t)h > edge);
    const int whole = !width && !height && !preview;
    if (w < 1 || h < 1 || w > MAX_EDGE || h > MAX_EDGE || bands < 1 || bands > 5 || sample == 0 ||
        (uint64_t)w * h > MAX_PIXELS) {
        fail(error, capacity, "This file reports an impossible image size"); goto done;
    }
    if (preview && (uint64_t)w * h > MAX_PREVIEW_PIXELS) {
        fail(error, capacity, "This image is too large to preview"); goto done;
    }
    output->source_sample_bytes = (uint32_t)sample;
    output->source_width = (uint32_t)w;
    output->source_height = (uint32_t)h;
    /* The decoded source and the RGBA result, at most 16 bits a sample. */
    const uint64_t needed = (uint64_t)w * h * (bands * sample + 4 * (sample > 1 ? 2 : 1));
    if (whole && needed > max_bytes) {
        output->needed = needed;
        fail(error, capacity, "Opening this image at full size needs more memory than allowed"); goto done;
    }
    if (vips_image_get_typeof(header, VIPS_META_ICC_NAME)) {
        if (vips_image_get_blob(header, VIPS_META_ICC_NAME, &profile, &profile_length)) goto native_error;
        if (profile_length > MAX_PROFILE ||
            !vips_icc_is_compatible_profile(header, profile, profile_length)) {
            fail(error, capacity, "Embedded color profile is invalid or does not match image channels"); goto done;
        }
        /* libvips may silently substitute a default profile if the requested
         * intent is unsupported. Reject it explicitly before asking to render. */
        icc = cmsOpenProfileFromMem(profile, (cmsUInt32Number)profile_length);
        if (!icc || !cmsIsIntentSupported(icc, INTENT_RELATIVE_COLORIMETRIC, LCMS_USED_AS_INPUT)) {
            fail(error, capacity, "Embedded color profile does not support relative color conversion"); goto done;
        }
        output->profiled = 1;
    }
    if (cancelled(context)) { fail(error, capacity, "Image preparation cancelled"); goto done; }
    if (width || height) {
        uint32_t ow = orientation >= 5 ? (uint32_t)h : (uint32_t)w;
        uint32_t oh = orientation >= 5 ? (uint32_t)w : (uint32_t)h;
        if (!width || !height || orientation < 1 || orientation > 8 || tile_edge > 4096 ||
            (uint64_t)x + width > ow || (uint64_t)y + height > oh) {
            fail(error, capacity, "Inspection region is outside the source image"); goto done;
        }
        /* Expand only within the oriented image. Crop before rotating, so a
         * small viewport never constructs a full rotated raster. The caller
         * chooses the grid and owns cache reuse and its memory budget. */
        if (tile_edge) {
            uint32_t right = MIN(ow, ((x + width + tile_edge - 1) / tile_edge) * tile_edge);
            uint32_t bottom = MIN(oh, ((y + height + tile_edge - 1) / tile_edge) * tile_edge);
            x = (x / tile_edge) * tile_edge; y = (y / tile_edge) * tile_edge;
            width = right - x; height = bottom - y;
        }
        uint32_t sx=x, sy=y, sw=width, sh=height;
        switch (orientation) {
            case 2: sx=w-x-width; break;
            case 3: sx=w-x-width; sy=h-y-height; break;
            case 4: sy=h-y-height; break;
            case 5: sx=y; sy=x; sw=height; sh=width; break;
            case 6: sx=y; sy=h-x-width; sw=height; sh=width; break;
            case 7: sx=w-y-height; sy=h-x-width; sw=height; sh=width; break;
            case 8: sx=w-y-height; sy=x; sw=height; sh=width; break;
        }
        if (vips_crop(header, &working, sx, sy, sw, sh, NULL)) goto native_error;
    } else if (edge && ((uint32_t)w > edge || (uint32_t)h > edge)) {
        /* Let native loaders shrink before decoding whenever supported. Source
         * headers are inspected above, but no source pixels were evaluated.
         * vips_thumbnail keeps at least 2x for its final reduce, so a 24 MP
         * JPEG would decode at full size for a 2048 px preview. Instead decode
         * with libjpeg-turbo's scaled IDCT at the largest 1/2, 1/4 or 1/8 that
         * still covers the edge, then reduce once. */
        const char *loader = NULL;
        int shrink = 1;
        if (!vips_image_get_string(header, VIPS_META_LOADER, &loader) && loader &&
            g_str_has_prefix(loader, "jpegload")) {
            uint32_t longest = (uint32_t)MAX(w, h);
            for (int s = 8; s > 1; s /= 2)
                if (longest / (uint32_t)s >= edge) { shrink = s; break; }
        }
        if (shrink > 1) {
            VipsImage *shrunk = NULL;
            if (vips_jpegload_source(source, &shrunk, "shrink", shrink,
                "access", VIPS_ACCESS_SEQUENTIAL, "fail_on", VIPS_FAIL_ON_WARNING, NULL))
                goto native_error;
            int failed = vips_thumbnail_image(shrunk, &working, edge,
                "height", edge, "size", VIPS_SIZE_DOWN, "no_rotate", TRUE,
                "output_profile", "srgb", "intent", VIPS_INTENT_RELATIVE, NULL);
            g_object_unref(shrunk);
            if (failed) goto native_error;
        } else if (vips_thumbnail_source(source, &working, edge,
            "height", edge, "size", VIPS_SIZE_DOWN, "no_rotate", TRUE,
            "output_profile", "srgb", "intent", VIPS_INTENT_RELATIVE,
            "fail_on", VIPS_FAIL_ON_WARNING, NULL)) goto native_error;
        colored = g_object_ref(working);
    } else working = g_object_ref(header);
    if (!colored) {
        if (output->profiled) {
            if (vips_icc_transform(working, &colored, "srgb", "embedded", TRUE,
                "intent", VIPS_INTENT_RELATIVE, "depth", sample > 1 ? 16 : 8, NULL)) goto native_error;
        } else if (vips_colourspace(working, &colored, sample > 1 && !edge ? VIPS_INTERPRETATION_RGB16 : VIPS_INTERPRETATION_sRGB, NULL)) goto native_error;
    }
    if (edge && (vips_image_get_width(colored) > (int)edge || vips_image_get_height(colored) > (int)edge)) {
        if (vips_thumbnail_image(colored, &sized, edge, "height", edge,
            "size", VIPS_SIZE_DOWN, "no_rotate", TRUE, NULL)) goto native_error;
    } else sized = g_object_ref(colored);
    /* UI/analysis previews use straight-alpha RGBA8. Full-resolution sources
     * retain 16-bit integer precision when supplied by the decoder. */
    VipsBandFormat format = !edge && sample > 1 ? VIPS_FORMAT_USHORT : VIPS_FORMAT_UCHAR;
    if (vips_cast(sized, &cast, format, "shift", TRUE, NULL)) goto native_error;
    if (!vips_image_hasalpha(cast)) {
        if (vips_addalpha(cast, &rgba, NULL)) goto native_error;
    } else rgba = g_object_ref(cast);
    if (vips_image_get_bands(rgba) != 4) {
        fail(error, capacity, "Image cannot be represented as RGBA"); goto done;
    }
    if (orientation != 1) {
        /* Rotation/mirroring may request scanlines backwards. Materialize
         * only the requested crop after any display-size reduction. */
        vips_image_set_progress(rgba, TRUE);
        gulong region_signal = g_signal_connect(rgba, "eval", G_CALLBACK(progress), &p);
        VipsImage *memory = vips_image_copy_memory(rgba);
        g_signal_handler_disconnect(rgba, region_signal);
        if (!memory) goto native_error;
        VIPS_UNREF(rgba); rgba = memory;
        if (cancelled(context)) { fail(error, capacity, "Image preparation cancelled"); goto done; }
        VipsAngle angle = orientation == 3 ? VIPS_ANGLE_D180 :
            orientation == 8 ? VIPS_ANGLE_D270 :
            orientation >= 5 ? VIPS_ANGLE_D90 : VIPS_ANGLE_D0;
        if (angle != VIPS_ANGLE_D0) {
            VipsImage *rotated = NULL;
            if (vips_rot(rgba, &rotated, angle, NULL)) goto native_error;
            VIPS_UNREF(rgba); rgba = rotated;
        }
        if (orientation == 2 || orientation == 4 || orientation == 5 || orientation == 7) {
            VipsImage *flipped = NULL;
            VipsDirection direction = orientation == 4 || orientation == 7 ?
                VIPS_DIRECTION_VERTICAL : VIPS_DIRECTION_HORIZONTAL;
            if (vips_flip(rgba, &flipped, direction, NULL)) goto native_error;
            VIPS_UNREF(rgba); rgba = flipped;
        }
    }
    output->width = vips_image_get_width(rgba);
    output->height = vips_image_get_height(rgba);
    output->sample_bytes = format == VIPS_FORMAT_USHORT ? 2 : 1;
    uint64_t expected = (uint64_t)output->width * output->height * 4 * output->sample_bytes;
    if (expected > max_bytes) { fail(error, capacity, "Output needs more memory than allowed"); goto done; }
    vips_image_set_progress(rgba, TRUE);
    signal = g_signal_connect(rgba, "eval", G_CALLBACK(progress), &p);
    output->pixels = vips_image_write_to_memory(rgba, &output->length);
    if (!output->pixels) goto native_error;
    if (cancelled(context)) { fail(error, capacity, "Image preparation cancelled"); goto done; }
    if (output->length != expected) { fail(error, capacity, "Native image buffer has unexpected length"); goto done; }
    result = 0;
    goto done;
native_error:
    native_failure(error, capacity);
done:
    if (signal) g_signal_handler_disconnect(rgba, signal);
    VIPS_UNREF(rgba); VIPS_UNREF(cast); VIPS_UNREF(sized); VIPS_UNREF(colored);
    VIPS_UNREF(working); VIPS_UNREF(header); VIPS_UNREF(source);
    if (icc) cmsCloseProfile(icc);
    if (result) {
        /* A refusal for memory still says what the render would need. */
        const uint64_t needed = output->needed;
        g_free(output->pixels); memset(output, 0, sizeof(*output));
        output->needed = needed;
    }
    vips_error_clear();
    g_private_set(&current_warnings, previous_warnings);
    return result;
}
