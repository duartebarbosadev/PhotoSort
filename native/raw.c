#include <libraw/libraw.h>
/* Keep LibRaw's version-specific structs on the C side of a tiny owned boundary. */
static int source_geometry(libraw_data_t *raw, unsigned int *width, unsigned int *height) {
    raw->params.user_flip = 0;
    int error = libraw_adjust_sizes_info_only(raw);
    if (!error && raw->sizes.iwidth && raw->sizes.iheight) {
        *width = raw->sizes.iwidth;
        *height = raw->sizes.iheight;
    }
    return error;
}
libraw_processed_image_t *photosort_raw_decode(const char *path, int recipe, int *embedded,
    unsigned int *source_width, unsigned int *source_height,
    progress_callback progress, void *context, int *error) {
    *embedded = 0;
    *source_width = *source_height = 0;
    const int preview = recipe != 0;
    libraw_data_t *raw = libraw_init(0);
    if (!raw) { *error = LIBRAW_UNSUFFICIENT_MEMORY; return 0; }
    libraw_set_progress_handler(raw, progress, context);
    *error = libraw_open_file(raw, path);
    if (*error) { libraw_close(raw); return 0; }
    if ((unsigned long long)raw->sizes.raw_width * raw->sizes.raw_height > 100000000ULL) {
        *error = LIBRAW_UNSUFFICIENT_MEMORY; libraw_close(raw); return 0;
    }
    if (recipe == 1) {
        *error = libraw_unpack_thumb(raw);
        if (!*error) {
            libraw_processed_image_t *thumb = libraw_dcraw_make_mem_thumb(raw, error);
            if (thumb && thumb->type == LIBRAW_IMAGE_JPEG) {
                /* This geometry-only call prevents subsequent unpack/process on this
                   handle. After extracting a JPEG, no further sensor decode is needed. */
                *error = source_geometry(raw, source_width, source_height);
                if (!*error && *source_width && *source_height) {
                    *embedded = 1;
                    libraw_close(raw);
                    return thumb;
                }
                libraw_dcraw_clear_mem(thumb);
                libraw_close(raw);
                if (*error == LIBRAW_CANCELLED_BY_CALLBACK) return 0;
                return photosort_raw_decode(path, 2, embedded, source_width, source_height,
                    progress, context, error);
            }
            if (thumb) libraw_dcraw_clear_mem(thumb);
        }
        if (*error == LIBRAW_CANCELLED_BY_CALLBACK) { libraw_close(raw); return 0; }
        *error = 0; /* Missing/non-JPEG thumbnails use neutral half-size decoding. */
    }
    if (preview) {
        /* Fallback needs both full-source geometry and a reduced sensor decode.
           LibRaw's info-only call advances processing state, so use a short-lived
           header-only handle here. No image/model work is repeated. */
        libraw_data_t *geometry = libraw_init(0);
        if (geometry) {
            libraw_set_progress_handler(geometry, progress, context);
            int geometry_error = libraw_open_file(geometry, path);
            if (!geometry_error) geometry_error = source_geometry(geometry, source_width, source_height);
            libraw_close(geometry);
            if (geometry_error == LIBRAW_CANCELLED_BY_CALLBACK) {
                *error = geometry_error; libraw_close(raw); return 0;
            }
        }
    }
    const int geometry_known = *source_width && *source_height;
    raw->params.use_camera_wb = 1;
    raw->params.output_color = 1;
    raw->params.output_bps = 8;
    raw->params.user_qual = 3;
    raw->params.user_flip = 0; /* EXIF/XMP orientation belongs to the shared pipeline. */
    raw->params.half_size = preview && geometry_known ? 1 : 0;
    raw->params.no_auto_bright = preview ? 1 : 0;
    raw->params.auto_bright_thr = 0.01;
    raw->params.bright = preview ? 1.0 : 1.25;
    raw->params.highlight = 0;
    raw->params.gamm[0] = 1.0 / 2.222;
    raw->params.gamm[1] = 4.5;
    *error = libraw_unpack(raw);
    if (!*error) *error = libraw_dcraw_process(raw);
    libraw_processed_image_t *image = *error ? 0 : libraw_dcraw_make_mem_image(raw, error);
    if (image && (!preview || !geometry_known)) {
        *source_width = image->width;
        *source_height = image->height;
    }
    libraw_close(raw);
    return image;
}
