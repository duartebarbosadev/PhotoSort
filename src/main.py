import pyexiv2 # This must be the first import or else it will cause a silent crash on windows - Don't ask me how many hours I spent until I found this...
import sys
import os
import logging # Added for startup logging
import time # Added for startup timing
import traceback # For global exception handler
from PyQt6.QtWidgets import QApplication, QMessageBox # QMessageBox for global exception handler
from src.ui.main_window import MainWindow
from src.core.metadata_processor import MetadataProcessor # For metadata processing
from src.ui.app_controller import AppController

def load_stylesheet(filename="src/ui/dark_theme.qss"):
    """Loads an external QSS stylesheet."""
    try:
        # Construct path relative to this script's directory or project root
        # This assumes main.py is run from the project root directory
        style_path = os.path.join(os.path.dirname(__file__), '..', filename) # Go up one level from src
        style_path = os.path.abspath(style_path) # Get absolute path

        if not os.path.exists(style_path):
             # Fallback if running from src directory itself? Less likely.
             style_path = filename
             if not os.path.exists(style_path):
                  logging.warning(f"Stylesheet '{filename}' not found at expected paths.")
                  return "" # Return empty string if not found

        logging.info(f"Loading stylesheet from: {style_path}")
        with open(style_path, "r") as f:
            return f.read()
    except Exception as e:
        logging.error(f"Error loading stylesheet '{filename}': {e}")
        return "" # Return empty on error

# --- Global Exception Handler ---
def global_exception_handler(exc_type, exc_value, exc_traceback):
    """Handles any unhandled exception, logs it, and shows an error dialog."""
    # Format the traceback
    error_message_details = "".join(traceback.format_exception(exc_type, exc_value, exc_traceback))
    
    # Log the critical error
    logging.critical(f"Unhandled Exception:\n{error_message_details}")
    
    # Attempt to show a user-friendly dialog
    app_instance = QApplication.instance() # Check if QApplication exists
    
    # Construct a simpler message for the main part of the dialog
    main_error_text = f"A critical error occurred: {str(exc_value)}\n\n" \
                      "The application may become unstable or need to close.\n" \
                      "Please report this error with the details provided."

    if app_instance:
        try:
            # QMessageBox is imported at the top level
            error_box = QMessageBox()
            error_box.setIcon(QMessageBox.Icon.Critical)
            error_box.setWindowTitle("Application Error")
            error_box.setText(main_error_text)
            error_box.setDetailedText(error_message_details) # Full traceback for expert users/reporting
            error_box.setStandardButtons(QMessageBox.StandardButton.Ok)
            error_box.exec()
        except Exception as e_msgbox:
            logging.error(f"Failed to show QMessageBox for unhandled exception: {str(e_msgbox)}\n"
                          f"Original error was:\n{error_message_details}")
            # Fallback to stderr if QMessageBox fails
            logging.critical(f"Unhandled exception (QMessageBox failed):\n{error_message_details}")
    else:
        # QApplication not yet initialized or already destroyed, print to stderr
        logging.critical(f"Unhandled exception (QApplication not available):\n{main_error_text}\nDetails:\n{error_message_details}")
    
    # Python will typically terminate after an unhandled exception and its excepthook.

# --- End Global Exception Handler ---

import argparse

def main():
    """Main application entry point."""

    # --- Enable Faulthandler for crash analysis ---
    import faulthandler
    faulthandler.enable()

    # Parse command-line arguments
    parser = argparse.ArgumentParser(description='PhotoSort')
    parser.add_argument('--folder', type=str, help='Open specified folder at startup')
    parser.add_argument('--clear-cache', action='store_true', help='Clear all caches before starting')
    args = parser.parse_args()

    # --- Aggressive Logging Setup ---
    # Get the root logger
    root_logger = logging.getLogger()

    # Remove any existing handlers
    for handler in root_logger.handlers[:]: # Iterate over a copy
        root_logger.removeHandler(handler)
        handler.close() # Important to close handlers to release resources

    # Create our desired formatter
    formatter = logging.Formatter(
        fmt='%(asctime)s - %(levelname)s - [%(filename)s:%(lineno)d] - %(message)s',
        datefmt='%Y-%m-%d %H:%M:%S'
    )

    # Create a new stream handler (outputs to console)
    console_handler = logging.StreamHandler(sys.stderr) # or sys.stdout
    console_handler.setFormatter(formatter)
    console_handler.setLevel(logging.DEBUG) # Set level for console
    root_logger.addHandler(console_handler)

    # Conditionally create and add a file handler
    enable_file_logging_env = os.environ.get("PHOTOSORT_ENABLE_FILE_LOGGING", "false")
    if enable_file_logging_env.lower() == "true":
        try:
            log_file_path = os.path.join(os.path.expanduser('~'), '.photosort_logs', 'photosort_app.log')
            os.makedirs(os.path.dirname(log_file_path), exist_ok=True)
            file_handler = logging.FileHandler(log_file_path, mode='a') # Append mode
            file_handler.setFormatter(formatter)
            file_handler.setLevel(logging.DEBUG) # Log DEBUG and above to file
            root_logger.addHandler(file_handler)
            root_logger.setLevel(logging.DEBUG) # Ensure root logger captures DEBUG for file handler
            logging.info(f"File logging ENABLED and initialized at: {log_file_path}")
        except Exception as e_file_log:
            logging.error(f"Failed to initialize file logging: {e_file_log}")
            root_logger.setLevel(logging.INFO) # Fallback to INFO if file logging fails
    else:
        root_logger.setLevel(logging.DEBUG) # Default to DEBUG if file logging is not enabled
        logging.info("File logging is DISABLED (set PHOTOSORT_ENABLE_FILE_LOGGING=true to enable).")
    
        # --- Suppress verbose third-party loggers ---
    logging.getLogger('PIL').setLevel(logging.INFO)
    logging.getLogger('PIL.PngImagePlugin').setLevel(logging.INFO)
    logging.getLogger('PIL.TiffImagePlugin').setLevel(logging.INFO)
    # You might also want to set it for the more general Image module if logs still appear
    logging.getLogger('PIL.Image').setLevel(logging.INFO)
    # --- End Suppress verbose third-party loggers ---

    # --- Setup Global Exception Hook ---
    sys.excepthook = global_exception_handler # Assign the function
    logging.info("Global exception hook (sys.excepthook) set.")
    # --- End Global Exception Hook Setup ---

    
    main_start_time = time.perf_counter()
    logging.info("Application main - Start")

    # Handle clear-cache argument
    if args.clear_cache:
        clear_application_caches_start_time = time.perf_counter()
        AppController.clear_application_caches()
        logging.info(f"Application main - clear_application_caches (via --clear-cache): {time.perf_counter() - clear_application_caches_start_time:.4f}s")


    app_instantiation_start_time = time.perf_counter()
    app = QApplication(sys.argv)
    logging.info(f"Application main - QApplication instantiated: {time.perf_counter() - app_instantiation_start_time:.4f}s")

    # Load and apply the stylesheet
    stylesheet_load_start_time = time.perf_counter()
    stylesheet = load_stylesheet()
    if (stylesheet):
        app.setStyleSheet(stylesheet)
        logging.info(f"Application main - Stylesheet loaded and applied: {time.perf_counter() - stylesheet_load_start_time:.4f}s")
    else:
        logging.warning("Stylesheet not loaded, using default application style.")
        logging.info(f"Application main - Stylesheet loading attempted (not found/error): {time.perf_counter() - stylesheet_load_start_time:.4f}s")


    mainwindow_instantiation_start_time = time.perf_counter()
    window = MainWindow(initial_folder=args.folder)
    logging.info(f"Application main - MainWindow instantiated: {time.perf_counter() - mainwindow_instantiation_start_time:.4f}s")

    window_show_start_time = time.perf_counter()
    window.show()
    logging.info(f"Application main - window.show() called: {time.perf_counter() - window_show_start_time:.4f}s")

    # Clear caches on exit
    # app.aboutToQuit.connect(clear_application_caches) # Prevent clearing caches on exit for persistence
    logging.info("Application main - aboutToQuit connection to clear_application_caches SKIPPED for persistence")

    # --- Stop ExifTool Process on Exit ---
    # Ensure clean shutdown (if using persistent handler)
    # app.aboutToQuit.connect(RatingHandler.stop_exiftool) # Uncomment if using persistent handler

    logging.info(f"Application main - Entering app.exec(). Total setup time: {time.perf_counter() - main_start_time:.4f}s")
    exit_code = app.exec()
    logging.info(f"Application main - Exited app.exec() with code {exit_code}. Total runtime: {time.perf_counter() - main_start_time:.4f}s")
    sys.exit(exit_code)

if __name__ == "__main__":
    main()