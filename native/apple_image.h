#ifndef PHOTOSORT_APPLE_IMAGE_H
#define PHOTOSORT_APPLE_IMAGE_H
unsigned char *photosort_apple_decode(const unsigned char *path, long path_len, unsigned int edge,
    unsigned int *width, unsigned int *height, unsigned int *source_width, unsigned int *source_height);
void photosort_apple_free(unsigned char *pixels);
unsigned int photosort_apple_orientation(const unsigned char *path, long path_len);
#endif
