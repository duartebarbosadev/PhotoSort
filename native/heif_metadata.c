#include <libheif/heif.h>
#include <libheif/heif_properties.h>
#include <stdint.h>

/* The safe Rust binding does not expose transformative properties. Keep only
   this metadata query at the C boundary; libheif owns parsing and ordering. */
int photosort_heif_transforms(const char *path, int32_t *operations, int capacity) {
    if (!path || !operations || capacity < 1 || capacity > 32) return -1;
    struct heif_context *context = heif_context_alloc();
    if (!context) return -1;
    struct heif_security_limits *limits = heif_context_get_security_limits(context);
    limits->max_image_size_pixels = 100000000;
    limits->max_color_profile_size = 4 * 1024 * 1024;
    limits->max_memory_block_size = 512ULL * 1024 * 1024;
    limits->max_total_memory = 512ULL * 1024 * 1024;
    int result = -1;
    if (heif_context_read_from_file(context, path, NULL).code != heif_error_Ok) goto done;
    heif_item_id item;
    if (heif_context_get_primary_image_ID(context, &item).code != heif_error_Ok) goto done;
    int count = heif_item_get_transformation_properties(context, item, NULL, 0);
    if (count < 0 || count > capacity) goto done;
    heif_property_id properties[32];
    if (heif_item_get_transformation_properties(context, item, properties, capacity) != count) goto done;
    result = 0;
    for (int i = 0; i < count; ++i) {
        enum heif_item_property_type type = heif_item_get_property_type(context, item, properties[i]);
        int operation;
        if (type == heif_item_property_type_transform_rotation) {
            operation = heif_item_get_property_transform_rotation_ccw(context, item, properties[i]);
            if (operation < 0 || operation > 270 || operation % 90) { result = -1; goto done; }
        } else if (type == heif_item_property_type_transform_mirror) {
            enum heif_transform_mirror_direction direction = heif_item_get_property_transform_mirror(context, item, properties[i]);
            if (direction == heif_transform_mirror_direction_horizontal) operation = -1;
            else if (direction == heif_transform_mirror_direction_vertical) operation = -2;
            else { result = -1; goto done; }
        } else if (type == heif_item_property_type_transform_crop) {
            continue; /* The decoder applies the crop in its proper sequence. */
        } else { result = -1; goto done; }
        operations[result++] = operation;
    }
done:
    heif_context_free(context);
    return result;
}
