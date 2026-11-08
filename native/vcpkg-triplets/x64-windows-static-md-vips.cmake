# libvips for the native-imaging engine (crates/imaging-vips): static libraries and the
# dynamic CRT, like x64-windows-static-md, but installed in a folder of their own. vcpkg's
# static GLib has the same library name as the GStreamer GLib the application links, so
# only photosort_vips.dll links from here (see crates/imaging-vips/build.rs).
set(VCPKG_TARGET_ARCHITECTURE x64)
set(VCPKG_CRT_LINKAGE dynamic)
set(VCPKG_LIBRARY_LINKAGE static)
# photosort_vips.dll links release libraries in every Cargo profile.
set(VCPKG_BUILD_TYPE release)

if(PORT STREQUAL "libvips")
    # Options scripts/build_vips.py builds with that the port has no feature for (Meson
    # takes the last -D it is given): GIF through libvips' bundled libnsgif, and Highway
    # SIMD, without which resizing a 24 MP photo takes several times longer. Cargo.toml
    # installs highway for this triplet; if it is missing, configuring libvips fails.
    set(VCPKG_MESON_CONFIGURE_OPTIONS -Dnsgif=true -Dhighway=enabled)
endif()
