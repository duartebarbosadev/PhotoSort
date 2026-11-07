#include <libraw/libraw.h>
#ifdef _WIN32
#include <stdlib.h>
#include <windows.h>
#endif
/* Rust passes UTF-8 paths. LibRaw's narrow open uses the ANSI code page on
   Windows, so photos in folders with accented names would not open; use its
   wide-character open there. */
static int open_path(libraw_data_t *raw, const char *path) {
#ifdef _WIN32
    int length = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1, NULL, 0);
    if (length > 0) {
        wchar_t *wide = (wchar_t *)malloc((size_t)length * sizeof(wchar_t));
        if (!wide) return LIBRAW_UNSUFFICIENT_MEMORY;
        MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1, wide, length);
        int error = libraw_open_wfile(raw, wide);
        free(wide);
        return error;
    }
#endif
    return libraw_open_file(raw, path);
}
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
#include <limits.h>
/* Returned with *needed when a sensor decode would take more than its allowance. */
#define PHOTOSORT_NEEDS_MEMORY (-200001)
/* Returned when LibRaw could not size the result safely, whatever the memory. */
#define PHOTOSORT_IMPOSSIBLE_SIZE (-200002)
/* Peak bytes per sensor pixel: the 16-bit mosaic, LibRaw's four 16-bit
   channels, the 8-bit RGB result and the caller's copy and rotation of it.
   A half-size decode keeps the mosaic but only a quarter of the rest. */
#define FULL_BYTES_PER_PIXEL 18
#define HALF_BYTES_PER_PIXEL 5
libraw_processed_image_t *photosort_raw_decode(const char *path, int recipe,
    unsigned long long allowance, unsigned long long *needed, int *embedded,
    unsigned int *source_width, unsigned int *source_height,
    progress_callback progress, void *context, int *error) {
    *needed = 0;
    *embedded = 0;
    *source_width = *source_height = 0;
    const int preview = recipe != 0;
    libraw_data_t *raw = libraw_init(0);
    if (!raw) { *error = LIBRAW_UNSUFFICIENT_MEMORY; return 0; }
    libraw_set_progress_handler(raw, progress, context);
    *error = open_path(raw, path);
    if (*error) { libraw_close(raw); return 0; }
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
                return photosort_raw_decode(path, 2, allowance, needed, embedded,
                    source_width, source_height, progress, context, error);
            }
            if (thumb) libraw_dcraw_clear_mem(thumb);
        }
        if (*error == LIBRAW_CANCELLED_BY_CALLBACK) { libraw_close(raw); return 0; }
        *error = 0; /* Missing/non-JPEG thumbnails use neutral half-size decoding. */
    }
    {
        /* The full-size output geometry: a preview fallback reports it, and
           every sensor decode is bounded by it below. LibRaw's info-only call
           advances processing state, so use a short-lived header-only handle
           here. No image/model work is repeated. */
        libraw_data_t *geometry = libraw_init(0);
        if (!geometry) { *error = LIBRAW_UNSUFFICIENT_MEMORY; libraw_close(raw); return 0; }
        libraw_set_progress_handler(geometry, progress, context);
        int geometry_error = open_path(geometry, path);
        if (!geometry_error) geometry_error = source_geometry(geometry, source_width, source_height);
        libraw_close(geometry);
        if (geometry_error == LIBRAW_CANCELLED_BY_CALLBACK) {
            *error = geometry_error; libraw_close(raw); return 0;
        }
    }
    /* Only a sensor decode holds the whole mosaic, so only it is bounded by
       memory; an embedded JPEG above is returned for any sensor size. */
    const unsigned long long sensor =
        (unsigned long long)raw->sizes.raw_width * raw->sizes.raw_height;
    /* Pixel aspect stretching can make the result several times the sensor,
       so a decode whose result size is unknown is not started. */
    if (!*source_width || !*source_height) {
        *error = PHOTOSORT_IMPOSSIBLE_SIZE;
        *source_width = raw->sizes.raw_width;
        *source_height = raw->sizes.raw_height;
        libraw_close(raw); return 0;
    }
    const unsigned long long full_output =
        (unsigned long long)*source_width * *source_height * 3;
    /* LibRaw sizes its 8-bit RGB result and its row offsets in 32-bit ints,
       so neither the sensor nor the full-size result (after pixel aspect
       stretching or Fuji's rotation) may pass INT_MAX bytes as RGB, whatever
       memory allows. A 400 MP pixel-shift file is about 1.2 GB; only a
       crafted header comes near this. */
    if (sensor * 3 > INT_MAX || full_output > INT_MAX) {
        *error = PHOTOSORT_IMPOSSIBLE_SIZE;
        libraw_close(raw); return 0;
    }
    *needed = sensor * (preview ? HALF_BYTES_PER_PIXEL : FULL_BYTES_PER_PIXEL);
    if (!sensor || *needed > allowance) {
        *error = sensor ? PHOTOSORT_NEEDS_MEMORY : LIBRAW_UNSUFFICIENT_MEMORY;
        libraw_close(raw); return 0;
    }
    /* LibRaw's own mosaic cap (2 GB by default) follows the same allowance,
       so multi-shot and pixel-shift files are not refused below it. */
    const unsigned long long allowed_mb = allowance / (1024 * 1024);
    if (allowed_mb > raw->rawparams.max_raw_memory_mb)
        raw->rawparams.max_raw_memory_mb = allowed_mb > 0xFFFFFFFFULL ? 0xFFFFFFFFU : (unsigned)allowed_mb;
    raw->params.use_camera_wb = 1;
    raw->params.output_color = 1;
    raw->params.output_bps = 8;
    raw->params.user_qual = 3;
    raw->params.user_flip = 0; /* EXIF/XMP orientation belongs to the shared pipeline. */
    raw->params.half_size = preview;
    raw->params.no_auto_bright = preview ? 1 : 0;
    raw->params.auto_bright_thr = 0.01;
    raw->params.bright = preview ? 1.0 : 1.25;
    raw->params.highlight = 0;
    raw->params.gamm[0] = 1.0 / 2.222;
    raw->params.gamm[1] = 4.5;
    *error = libraw_unpack(raw);
    if (!*error) *error = libraw_dcraw_process(raw);
    libraw_processed_image_t *image = *error ? 0 : libraw_dcraw_make_mem_image(raw, error);
    if (image && !preview) {
        *source_width = image->width;
        *source_height = image->height;
    }
    libraw_close(raw);
    return image;
}
