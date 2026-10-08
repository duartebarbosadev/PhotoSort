#include <copyfile.h>
#include <errno.h>
#include <stddef.h>
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
    return result;
}
