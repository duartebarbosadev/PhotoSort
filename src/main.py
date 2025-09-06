import pyexiv2  # noqa: F401  # This must be the first import or else it will cause a silent crash on windows
import sys
import os
import logging
import time
import argparse
import traceback  # For global exception handler
from PyQt6.QtWidgets import (
    QApplication,
    QMessageBox,
)  # QMessageBox for global exception handler
from PyQt6.QtGui import QIcon

# Ensure the 'src' directory is on sys.path when executing as a script
SRC_DIR = os.path.dirname(__file__)
if SRC_DIR and SRC_DIR not in sys.path:
    sys.path.insert(0, SRC_DIR)

from ui.main_window import MainWindow  # noqa: E402
from ui.app_controller import AppController  # noqa: E402
from core.metadata_io import MetadataIO  # noqa: E402  # After pyexiv2 but before threads
from pillow_heif import register_heif_opener  # noqa: E402


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
            ),  # we bundle at top-level in frozen builds
            os.path.join(
                base_dir, filename
            ),  # e.g., src/ui/dark_theme.qss inside frozen or source
            os.path.abspath(
                filename
            ),  # direct path from CWD when running from repo root
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
            error_box.setDetailedText(
                error_message_details
            )  # Full traceback for expert users/reporting
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
        meipass = getattr(sys, "_MEIPASS", None)
        if meipass:
            return os.path.join(meipass, "app_icon.ico")
        if getattr(sys, "frozen", False):
            return os.path.join(os.path.dirname(sys.executable), "app_icon.ico")
        return os.path.join(
            os.path.dirname(os.path.dirname(__file__)), "assets", "app_icon.ico"
        )
    except Exception:
        return ""


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


def main():
    """Main application entry point."""

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

    register_heif_opener()

    # Perform early metadata backend warmup BEFORE creating QApplication or starting
    # any background workers that might touch pyexiv2 on Windows. This mitigates
    # rare access violations when the first Exiv2 interaction happens off the main thread.
    try:
        MetadataIO.warmup()
    except Exception as e:
        logging.warning(f"MetadataIO warmup failed (continuing): {e}")

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

    # Confirm pyexiv2 import (path and version) after logging is configured
    try:
        logging.info(
            "pyexiv2 loaded: path=%s, version=%s",
            getattr(pyexiv2, "__file__", "unknown"),
            getattr(pyexiv2, "__version__", "unknown"),
        )
    except Exception as _e_pe2:
        logging.warning(f"Failed to introspect pyexiv2 import details: {_e_pe2}")

    # Log current MetadataIO state now that logging is configured
    try:
        logging.debug(
            "MetadataIO state pre-Qt: warmed_up=%s, first_access_done=%s",
            getattr(MetadataIO, "_WARMED_UP", None),
            getattr(MetadataIO, "_FIRST_ACCESS_DONE", None),
        )
    except Exception:
        pass

        # --- Suppress verbose third-party loggers ---
    logging.getLogger("PIL").setLevel(logging.INFO)
    logging.getLogger("PIL.PngImagePlugin").setLevel(logging.INFO)
    logging.getLogger("PIL.TiffImagePlugin").setLevel(logging.INFO)
    # You might also want to set it for the more general Image module if logs still appear
    logging.getLogger("PIL.Image").setLevel(logging.INFO)
    # --- End Suppress verbose third-party loggers ---

    # Start the dedicated single-thread worker for all pyexiv2 calls on
    # Windows/frozen builds to avoid cross-thread access to C++ globals.
    try:
        MetadataIO.start_worker_thread()
    except Exception as e:
        logging.warning(
            f"MetadataIO worker thread start encountered an issue (continuing): {e}"
        )

    # --- Setup Global Exception Hook ---
    sys.excepthook = global_exception_handler  # Assign the function
    logging.debug("Global exception hook set.")
    # --- End Global Exception Hook Setup ---

    main_start_time = time.perf_counter()
    logging.info("Application starting...")

    # Handle clear-cache argument
    if args.clear_cache:
        clear_application_caches_start_time = time.perf_counter()
        AppController.clear_application_caches()
        logging.info(
            f"Caches cleared via command line in {time.perf_counter() - clear_application_caches_start_time:.4f}s"
        )

    app_instantiation_start_time = time.perf_counter()
    app = QApplication(sys.argv)
    logging.debug(
        f"QApplication instantiated in {time.perf_counter() - app_instantiation_start_time:.4f}s"
    )

    # This call is now a no-op (real access happens on dedicated worker thread)
    try:
        MetadataIO.ensure_first_access_main_thread()
    except Exception as e:
        logging.warning(
            f"MetadataIO ensure_first_access_main_thread failed (continuing): {e}"
        )

    # Load and apply the stylesheet
    stylesheet_load_start_time = time.perf_counter()
    stylesheet = load_stylesheet()
    if stylesheet:
        app.setStyleSheet(stylesheet)
        logging.debug(
            f"Stylesheet loaded and applied in {time.perf_counter() - stylesheet_load_start_time:.4f}s"
        )
    else:
        logging.warning("Stylesheet not found. Using default style.")

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
