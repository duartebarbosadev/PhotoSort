# Release Notes Generator

This directory contains scripts to automatically generate release notes using OpenAI's API.

## Files

- `generate_release_notes.py` - Main script that generates release notes using OpenAI based on git differences between releases

## How it works

1. When a new release is published on GitHub, the `release-notes-generator.yml` workflow triggers
2. The workflow runs the `generate_release_notes.py` script with the new release tag
3. The script:
   - Finds the previous release tag
   - Gets git differences (commits and file changes) between the releases
   - Sends the differences to OpenAI's API to generate user-friendly release notes
   - Falls back to basic notes if OpenAI fails
4. The workflow updates the GitHub release with the generated notes

## Configuration

The workflow requires an `OPENAI_API_KEY` secret to be set in the repository settings.

## Testing

Run the script locally:
```bash
python scripts/generate_release_notes.py v1.0.0 --api-key "your-api-key"
```

Run tests:
```bash
pytest tests/test_release_notes_generator.py
```