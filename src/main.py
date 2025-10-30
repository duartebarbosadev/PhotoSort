import sys
import os
import time
from typing import Optional

# Ensure the 'src' directory is on sys.path when executing as a script
SRC_DIR = os.path.dirname(__file__)
if SRC_DIR and SRC_DIR not in sys.path:
    sys.path.insert(0, SRC_DIR)

# Initialize pyexiv2 before any Qt imports - this is CRITICAL for Windows stability
try:
    from core.pyexiv2_init import ensure_pyexiv2_initialized  # noqa: E402

    ensure_pyexiv2_initialized()
except Exception as e:
    # If we can't initialize pyexiv2, log the error but don't prevent app startup
    print(f"Warning: Failed to initialize pyexiv2: {e}")

import logging  # noqa: E402
import argparse  # noqa: E402
import traceback  # noqa: E402  # For global exception handler

from PyQt6.QtCore import Qt, QTimer  # noqa: E402
from PyQt6.QtWidgets import QApplication, QMessageBox, QSplashScreen  # noqa: E402
from PyQt6.QtGui import QIcon, QPixmap  # noqa: E402


def load_stylesheet(filename: str = "src/ui/dark_theme.qss") -> str:
    """Load an external QSS stylesheet.

    Works in both source runs and frozen bundles (e.g., PyInstaller) by
    checking for the temporary extraction directory at runtime.
    """
    try:
        # Determine base directory depending on runtime context
        base_dir: str
        meipass = getattr(sys, "_MEIPASS", None)  # type: ignore[attr-defined]
        if meipass:
            base_dir = meipass  # PyInstaller onefile extraction dir
        elif getattr(sys, "frozen", False):  # PyInstaller onedir
            base_dir = os.path.dirname(sys.executable)
        else:
            # Running from source
            base_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))

        # Candidate locations, in order of preference
        candidates = [
            os.path.join(
                base_dir, "dark_theme.qss"
            ),  # bundled at top-level in frozen builds
            os.path.join(base_dir, filename),  # e.g., src/ui/dark_theme.qss
            os.path.abspath(filename),  # direct path from CWD
        ]

        for path in candidates:
            try:
                if os.path.exists(path) and os.path.isfile(path):
                    logging.info(f"Loading stylesheet: {path}")
                    with open(path, "r", encoding="utf-8") as f:
                        return f.read()
            except PermissionError as pe:
                logging.error(
                    f"Permission denied when reading stylesheet '{path}': {pe}"
                )
                continue
            except Exception as e_inner:
                logging.error(f"Failed to read stylesheet '{path}': {e_inner}")
                continue

        logging.warning(f"Stylesheet not found: searched {candidates}")
        return ""
    except Exception as e:
        logging.error(f"Failed to load stylesheet '{filename}': {e}")
        return ""


# --- Global Exception Handler ---
def global_exception_handler(exc_type, exc_value, exc_traceback):
    """Handles any unhandled exception, logs it, and shows an error dialog."""
    # Don't show a dialog for KeyboardInterrupt (Ctrl+C)
    if issubclass(exc_type, KeyboardInterrupt):
        logging.info("Application terminated by user.")
        # Let the default handler take over to exit
        sys.__excepthook__(exc_type, exc_value, exc_traceback)
        return

    # Format the traceback
    error_message_details = "".join(
        traceback.format_exception(exc_type, exc_value, exc_traceback)
    )

    # Log the critical error
    logging.critical(f"Unhandled exception occurred:\n{error_message_details}")

    # Attempt to show a user-friendly dialog
    app_instance = QApplication.instance()  # Check if QApplication exists

    # Construct a simpler message for the main part of the dialog
    main_error_text = (
        f"A critical error occurred: {str(exc_value)}\n\n"
        "The application may become unstable or need to close.\n"
        "Please report this error with the details provided."
    )

    if app_instance:
        try:
            # QMessageBox is imported at the top level
            error_box = QMessageBox()
            error_box.setIcon(QMessageBox.Icon.Critical)
            error_box.setWindowTitle("Application Error")
            error_box.setText(main_error_text)
            error_box.setDetailedText(error_message_details)  # Full traceback
            error_box.setStandardButtons(QMessageBox.StandardButton.Ok)
            error_box.exec()
        except Exception as e_msgbox:
            logging.error(
                f"Failed to display error dialog: {str(e_msgbox)}\nOriginal error:\n{error_message_details}"
            )
            # Fallback to stderr if QMessageBox fails
            logging.critical(
                f"Unhandled exception caught (dialog failed):\n{error_message_details}"
            )
    else:
        # QApplication not yet initialized or already destroyed, print to stderr
        logging.critical(
            f"Unhandled exception caught (QApplication not available):\n{error_message_details}"
        )


# --- App identity helpers (icon + Windows taskbar identity) ---
def _set_windows_app_id(app_id: str = "PhotoSort") -> None:
    """Set explicit AppUserModelID so Windows taskbar/pinned icon uses the app icon."""
    if sys.platform.startswith("win"):
        try:
            import ctypes

            func = ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID
            func.argtypes = [ctypes.c_wchar_p]
            func.restype = ctypes.c_long  # HRESULT
            hr = func(app_id)
            if hr != 0:  # S_OK == 0
                logging.warning(
                    f"SetCurrentProcessExplicitAppUserModelID failed, HRESULT=0x{hr & 0xFFFFFFFF:08X}"
                )
        except Exception as e_appid:
            logging.warning(f"Failed to set Windows AppUserModelID: {e_appid}")


def _resolve_app_icon_path() -> str:
    """Resolve the best path to the app icon across source and PyInstaller runs."""
    try:
        # Delegate to shared finder to keep resolution logic consistent
        candidate = _find_resource_path("app_icon.ico", include_exe_dir=True)
        return candidate or ""
    except Exception:
        return ""


def _find_resource_path(filename: str, include_exe_dir: bool = False) -> Optional[str]:
    """Find the first existing path for a given resource filename.

    Search order:
      1. PyInstaller _MEIPASS (if present)
      2. Executable directory (when frozen and include_exe_dir=True)
      3. Project `assets/` directory next to the package
      4. Current working directory (absolute)

    Returns the first existing file path, or None if not found.
    """
    meipass = getattr(sys, "_MEIPASS", None)

    candidates = []
    if meipass:
        candidates.append(os.path.join(meipass, filename))

    if include_exe_dir and getattr(sys, "frozen", False):
        candidates.append(os.path.join(os.path.dirname(sys.executable), filename))

    candidates.append(
        os.path.join(os.path.dirname(os.path.dirname(__file__)), "assets", filename)
    )
    candidates.append(os.path.abspath(filename))

    for candidate in candidates:
        try:
            if os.path.exists(candidate) and os.path.isfile(candidate):
                return candidate
        except Exception:
            # Ignore individual candidate errors; continue to next
            continue

    return None


def apply_app_identity(app: QApplication, main_window=None) -> None:
    """Apply Windows AppID and set icons on app and optionally the main window."""
    _set_windows_app_id()
    try:
        icon_path = _resolve_app_icon_path()
        if icon_path and os.path.exists(icon_path):
            icon = QIcon(icon_path)
            app.setWindowIcon(icon)
            if main_window is not None:
                try:
                    main_window.setWindowIcon(icon)
                except Exception:
                    pass
            logging.debug(f"Application icon set from: {icon_path}")
        else:
            logging.warning(f"Application icon not found at: {icon_path}")
    except Exception as e:
        logging.error(f"Failed to apply application icon: {e}")


def resolve_splash_logo_path() -> Optional[str]:
    """Resolve the best path to the splashscreen logo across source and PyInstaller runs.

    Returns the first existing candidate path, or None if no logo is found.
    """
    return _find_resource_path("app_icon.png", include_exe_dir=False)


def main():
    """Main application entry point."""

    # Create QApplication early for splash screen
    app = QApplication(sys.argv)

    # --- Splash: show immediately (no text), then set message after it’s visible ---
    splash_total_start = time.perf_counter()

    splash_path = resolve_splash_logo_path()
    if not splash_path:
        logging.warning("Splashscreen logo not found. Splash will be blank.")
        splash_pix = QPixmap()
    else:
        splash_pix = QPixmap(splash_path).scaled(
            400,
            300,
            Qt.AspectRatioMode.KeepAspectRatio,
            Qt.TransformationMode.FastTransformation,
        )
    splash = QSplashScreen(splash_pix)

    # Show the splash immediately (no text yet)
    splash.show()
    app.processEvents()  # should be fast; no text/layout yet

    from pillow_heif import register_heif_opener  # noqa: E402

    register_heif_opener()

    # --- Enable Faulthandler for crash analysis ---
    import faulthandler

    # In GUI builds (PyInstaller -w), sys.stderr can be None; enable to a file in that case
    try:
        if sys.stderr is not None:
            faulthandler.enable()
        else:
            try:
                crash_dir = os.path.join(os.path.expanduser("~"), ".photosort_logs")
                os.makedirs(crash_dir, exist_ok=True)
                crash_log_path = os.path.join(crash_dir, "photosort_crash.log")
                # Keep a global reference to avoid GC closing the file
                global _FAULTHANDLER_FH  # type: ignore[var-annotated]
                _FAULTHANDLER_FH = open(
                    crash_log_path, "a", buffering=1, encoding="utf-8"
                )
                faulthandler.enable(file=_FAULTHANDLER_FH)
                logging.info(f"Faulthandler crash log: {crash_log_path}")
            except Exception as fe:
                logging.error(f"Failed to set up faulthandler crash log: {fe}")
    except Exception:
        pass

    # Parse command-line arguments
    parser = argparse.ArgumentParser(description="PhotoSort")
    parser.add_argument("--folder", type=str, help="Open specified folder at startup")
    parser.add_argument(
        "--clear-cache", action="store_true", help="Clear all caches before starting"
    )
    args = parser.parse_args()

    # --- Aggressive Logging Setup ---
    # Get the root logger
    root_logger = logging.getLogger()

    # Remove any existing handlers
    for handler in root_logger.handlers[:]:  # Iterate over a copy
        root_logger.removeHandler(handler)
        handler.close()  # Important to close handlers to release resources

    # Create our desired formatter
    formatter = logging.Formatter(
        fmt="%(asctime)s - %(levelname)-8s - [%(name)s] - [%(filename)s:%(lineno)d] - %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )

    # Create a new stream handler (outputs to console) only if stderr exists
    if sys.stderr is not None:
        console_handler = logging.StreamHandler(sys.stderr)  # or sys.stdout
        console_handler.setFormatter(formatter)
        console_handler.setLevel(logging.DEBUG)  # Set level for console
        root_logger.addHandler(console_handler)

    # Conditionally create and add a file handler
    enable_file_logging_env = os.environ.get("PHOTOSORT_ENABLE_FILE_LOGGING", "false")
    # In GUI builds without a console, default to file logging on
    want_file_logging = enable_file_logging_env.lower() == "true" or sys.stderr is None
    if want_file_logging:
        try:
            log_file_path = os.path.join(
                os.path.expanduser("~"), ".photosort_logs", "photosort_app.log"
            )
            os.makedirs(os.path.dirname(log_file_path), exist_ok=True)
            file_handler = logging.FileHandler(log_file_path, mode="a")  # Append mode
            file_handler.setFormatter(formatter)
            file_handler.setLevel(logging.DEBUG)  # Log DEBUG and above to file
            root_logger.addHandler(file_handler)
            root_logger.setLevel(
                logging.DEBUG
            )  # Ensure root logger captures DEBUG for file handler
            logging.info(f"File logging enabled: {log_file_path}")
        except Exception as e_file_log:
            logging.error(
                f"Failed to initialize file logging: {e_file_log}", exc_info=True
            )
            root_logger.setLevel(logging.INFO)  # Fallback to INFO if file logging fails
    else:
        root_logger.setLevel(
            logging.DEBUG
        )  # Default to DEBUG if file logging is not enabled
        logging.info(
            "File logging disabled. To enable, set PHOTOSORT_ENABLE_FILE_LOGGING=true."
        )

    # --- Suppress verbose third-party loggers ---
    logging.getLogger("PIL").setLevel(logging.INFO)
    logging.getLogger("PIL.PngImagePlugin").setLevel(logging.INFO)
    logging.getLogger("PIL.TiffImagePlugin").setLevel(logging.INFO)
    logging.getLogger("PIL.Image").setLevel(logging.INFO)
    
    # Suppress verbose HTTP logging from OpenAI client and httpcore
    logging.getLogger("httpcore").setLevel(logging.WARNING)
    logging.getLogger("httpcore.http11").setLevel(logging.WARNING)
    logging.getLogger("httpcore.connection").setLevel(logging.WARNING)
    logging.getLogger("httpx").setLevel(logging.WARNING)
    logging.getLogger("openai").setLevel(logging.INFO)
    logging.getLogger("openai._base_client").setLevel(logging.WARNING)
    # --- End Suppress verbose third-party loggers ---

    # --- Setup Global Exception Hook ---
    sys.excepthook = global_exception_handler  # Assign the function
    logging.debug("Global exception hook set.")
    # --- End Global Exception Hook Setup ---

    main_start_time = time.perf_counter()
    logging.info("Application starting...")

    from ui.main_window import MainWindow
    from ui.app_controller import AppController

    # Handle clear-cache argument
    if args.clear_cache:
        clear_application_caches_start_time = time.perf_counter()
        AppController.clear_application_caches()
        logging.info(
            f"Caches cleared via command line in {time.perf_counter() - clear_application_caches_start_time:.4f}s"
        )

    mainwindow_instantiation_start_time = time.perf_counter()
    window = MainWindow(initial_folder=args.folder)
    apply_app_identity(app, window)
    logging.debug(
        f"MainWindow instantiated in {time.perf_counter() - mainwindow_instantiation_start_time:.4f}s"
    )

    window_show_start_time = time.perf_counter()
    window.show()
    logging.debug(
        f"MainWindow shown in {time.perf_counter() - window_show_start_time:.4f}s"
    )
    splash.finish(window)
    logging.debug(
        f"Splashscreen finished in {time.perf_counter() - splash_total_start:.4f}s"
    )

    # Defer stylesheet loading to after splash finish to avoid blocking startup
    stylesheet_load_start = time.perf_counter()

    def apply_stylesheet():
        stylesheet = load_stylesheet()
        if stylesheet:
            app.setStyleSheet(stylesheet)
        logging.debug(
            f"Stylesheet applied in {time.perf_counter() - stylesheet_load_start:.4f}s"
        )

    QTimer.singleShot(
        0,
        apply_stylesheet,
    )

    logging.info(
        f"Application setup complete in {time.perf_counter() - main_start_time:.4f}s. Entering event loop."
    )
    exit_code = app.exec()
    logging.info(
        f"Application exited with code {exit_code}. Total runtime: {time.perf_counter() - main_start_time:.4f}s"
    )
    sys.exit(exit_code)


if __name__ == "__main__":
    main()
