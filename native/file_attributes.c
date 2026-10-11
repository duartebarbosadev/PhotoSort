#include <copyfile.h>
#include <errno.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <sys/xattr.h>

struct photosort_copy_context {
    int (*cancelled)(void *);
    void *context;
    int error;
    int stopped;
};

static int photosort_copy_status(int what, int stage, copyfile_state_t state,
                                const char *source, const char *destination,
                                void *opaque) {
    (void)what;
    (void)state;
    (void)source;
    (void)destination;
    struct photosort_copy_context *context = opaque;
    if (stage == COPYFILE_ERR) {
        // Never tell copyfile to ignore an attribute-copy failure.
        if (!context->stopped && !context->error)
            context->error = errno ? errno : EIO;
        return COPYFILE_QUIT;
    }
    if (context->cancelled(context->context)) {
        context->stopped = 1;
        return COPYFILE_QUIT;
    }
    return COPYFILE_CONTINUE;
}

static int photosort_is_resource_fork(const char *name) {
    return strcmp(name, XATTR_RESOURCEFORK_NAME) == 0;
}

// Read one attribute whole. Returns its size, or -1 with errno set; *value is
// malloc'd (or NULL for an empty value) and owned by the caller. A resource
// fork can be returned in pieces, so it is read at increasing positions.
static ssize_t photosort_read_xattr(int fd, const char *name, char **value) {
    *value = NULL;
    for (int attempt = 0; attempt < 4; attempt++) {
        ssize_t size = fgetxattr(fd, name, NULL, 0, 0, 0);
        if (size <= 0) return size;
        char *buffer = malloc((size_t)size);
        if (!buffer) { errno = ENOMEM; return -1; }
        ssize_t total = 0;
        int fork = photosort_is_resource_fork(name);
        while (total < size) {
            ssize_t read = fgetxattr(fd, name, buffer + total, (size_t)(size - total),
                                     fork ? (u_int32_t)total : 0, 0);
            if (read <= 0 || (!fork && read != size)) { total = -1; break; }
            total += read;
            if (!fork) break;
        }
        if (total == size) { *value = buffer; return size; }
        int error = errno;
        free(buffer);
        if (error != ERANGE && error != 0) { errno = error; return -1; }
    }
    errno = EIO;
    return -1;
}

// Write one attribute whole; a resource fork is written in positioned pieces.
static int photosort_write_xattr(int fd, const char *name, const char *value, size_t size) {
    if (!photosort_is_resource_fork(name) || size == 0)
        return fsetxattr(fd, name, value ? value : "", size, 0, 0);
    const size_t piece = 1024 * 1024;
    for (size_t offset = 0; offset < size; offset += piece) {
        size_t count = size - offset < piece ? size - offset : piece;
        if (fsetxattr(fd, name, value + offset, count, (u_int32_t)offset, 0) != 0) return -1;
    }
    return 0;
}

// copyfile can return success without every attribute arriving (seen with
// large resource forks under heavy I/O). A move deletes its source, so every
// source attribute must be present and byte-identical on the destination;
// copy any that are not directly, then check again. 0, -1 cancelled, or errno.
static int photosort_verify_xattrs(int source, int destination,
                                   int (*cancelled)(void *), void *opaque) {
    ssize_t length = flistxattr(source, NULL, 0, 0);
    if (length <= 0) return length < 0 ? (errno ? errno : EIO) : 0;
    char *names = malloc((size_t)length);
    if (!names) return ENOMEM;
    length = flistxattr(source, names, (size_t)length, 0);
    if (length < 0) { int error = errno ? errno : EIO; free(names); return error; }
    int result = 0;
    for (char *name = names; result == 0 && name < names + length; name += strlen(name) + 1) {
        if (cancelled(opaque)) { result = -1; break; }
        char *expected = NULL, *actual = NULL;
        ssize_t want = photosort_read_xattr(source, name, &expected);
        if (want < 0) { result = errno ? errno : EIO; break; }
        for (int attempt = 0; attempt < 2; attempt++) {
            free(actual);
            ssize_t have = photosort_read_xattr(destination, name, &actual);
            if (have == want && (want == 0 || memcmp(expected, actual, (size_t)want) == 0)) {
                result = 0;
                break;
            }
            result = EIO;
            if (attempt == 0 &&
                photosort_write_xattr(destination, name, expected, (size_t)want) != 0) {
                result = errno ? errno : EIO;
                break;
            }
        }
        free(expected);
        free(actual);
    }
    free(names);
    return result;
}

// Borrow already-open descriptors; do not reopen paths, copy data, change
// permissions or remove either file. Return 0, -1 for cancellation, or errno.
int photosort_copy_xattrs(int source, int destination,
                         int (*cancelled)(void *), void *opaque) {
    // copyfile treats some unsupported/unreadable attribute lists as success.
    // A move must not remove a tagged source after that best-effort shortcut.
    ssize_t count = flistxattr(source, NULL, 0, 0);
    if (count < 0) return errno == ENOTSUP ? 0 : (errno ? errno : EIO);
    if (count == 0) return 0;
    if (flistxattr(destination, NULL, 0, 0) < 0)
        return errno ? errno : EIO;
    copyfile_state_t state = copyfile_state_alloc();
    if (!state) return errno ? errno : ENOMEM;
    struct photosort_copy_context context = {cancelled, opaque, 0, 0};
    int result;
    if (copyfile_state_set(state, COPYFILE_STATE_STATUS_CB, photosort_copy_status) ||
        copyfile_state_set(state, COPYFILE_STATE_STATUS_CTX, &context)) {
        result = errno ? errno : EIO;
    } else {
        int status = fcopyfile(source, destination, state, COPYFILE_XATTR);
        result = context.error ? context.error : context.stopped ? -1 :
                 status ? (errno ? errno : EIO) : 0;
    }
    copyfile_state_free(state);
    if (result == 0)
        result = photosort_verify_xattrs(source, destination, cancelled, opaque);
    return result;
}
