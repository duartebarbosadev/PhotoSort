/* Windows only. PhotoSort delay-loads GStreamer's GLib (the application starts
 * without it, and points Windows at it before decoding an image), but the linker
 * cannot delay-load a DLL whose variables a program imports. libvips reads two
 * GLib variables, through macros. Their import slots (the pointers that dllimport
 * code reads the variable's address from) are defined here instead of by GLib's
 * import libraries, and filled once GLib is loaded, before libvips runs. This
 * file is linked whole, so these definitions are present before the linker
 * searches GLib's import libraries. It does not include GLib's headers, which
 * declare the same names as imports. */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <string.h>

void *__imp_g_ascii_table;
void *__imp_g_param_spec_types;

int photosort_glib_data(char *error, size_t capacity) {
    HMODULE glib = LoadLibraryW(L"glib-2.0-0.dll");
    HMODULE gobject = LoadLibraryW(L"gobject-2.0-0.dll");
    if (glib) __imp_g_ascii_table = (void *)GetProcAddress(glib, "g_ascii_table");
    if (gobject) __imp_g_param_spec_types = (void *)GetProcAddress(gobject, "g_param_spec_types");
    if (__imp_g_ascii_table && __imp_g_param_spec_types) return 0;
    if (capacity) {
        strncpy(error, "GLib from GStreamer could not be loaded", capacity - 1);
        error[capacity - 1] = '\0';
    }
    return -1;
}
