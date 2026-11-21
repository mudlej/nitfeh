#!/bin/bash

# Default values
START_DIR="."
OUTPUT_DIR="./"
INCLUDE_AUDIO=false
LOG_FILE="./convert_improved.log"

# FFmpeg command (easily modifiable)
FFMPEG_CMD='ffmpeg -i "$input_file" -vn -ac 1 -ar 16000 -c:a libopus -b:a 12k "$output_file"'

# Function to display help
function display_help {
    echo "Usage: $0 [start_directory] [output_directory] [--audio] [--logfile logfile_path] [--help]"
    echo
    echo "Optional arguments:"
    echo "  start_directory    Directory to start searching for files (default: current directory)"
    echo "  output_directory   Directory to save the compressed files (default: current directory)"
    echo "  --audio            Include audio files in the compression"
    echo "  --logfile FILE     Redirect ffmpeg output to the specified log file (default: ./convert_improved.log)"
    echo "  --help             Display this help message and exit"
    echo
    echo "Note: The output of ffmpeg is redirected to the log file to reduce console noise."
    exit 0
}

# Parse arguments
while (( "$#" )); do
    case "$1" in
        --audio)
            INCLUDE_AUDIO=true
            shift
            ;;
        --logfile)
            if [ -n "$2" ] && [ "${2:0:1}" != "-" ]; then
                LOG_FILE="$2"
                shift 2
            else
                echo "Error: --logfile requires a non-empty option argument."
                exit 1
            fi
            ;;
        --help)
            display_help
            ;;
        *)
            if [ -z "$START_DIR_PROVIDED" ]; then
                START_DIR="$1"
                START_DIR_PROVIDED=true
            elif [ -z "$OUTPUT_DIR_PROVIDED" ]; then
                OUTPUT_DIR="$1"
                OUTPUT_DIR_PROVIDED=true
            else
                echo "Invalid argument: $1"
                display_help
            fi
            shift
            ;;
    esac
done

echo "FFmpeg output is being redirected to: $LOG_FILE"
echo "Output Directory is: $OUTPUT_DIR"

# List of file extensions to process
VIDEO_EXTENSIONS=("3gp" "asf" "avi" "flv" "m2ts" "m4v" "mkv" "mov" "mp4" "mpeg" "mpg" "mts" "ogv" "rmvb" "ts" "vob" "webm" "wmv")
AUDIO_EXTENSIONS=("aac" "flac" "m4a" "mp3" "ogg" "opus" "wav" "wma")

# Combine extensions based on --audio flag
if [ "$INCLUDE_AUDIO" = true ]; then
    EXTENSIONS=("${VIDEO_EXTENSIONS[@]}" "${AUDIO_EXTENSIONS[@]}")
else
    EXTENSIONS=("${VIDEO_EXTENSIONS[@]}")
fi

# Create an array to hold all files
files=()

# Find all files with the specified extensions recursively
for ext in "${EXTENSIONS[@]}"; do
    while IFS= read -r -d '' file; do
        files+=("$file")
    done < <(find -L "$START_DIR" -type f -iname "*.${ext}" -print0)
done

total_files=${#files[@]}
echo "Total files to process: $total_files"

# Check if any files are found
if [ "$total_files" -eq 0 ]; then
    echo "No files found with the specified extensions."
    exit 0
fi

# Initialize counters and start time
current_file=0
start_time=$(date +%s)

for input_file in "${files[@]}"; do
    current_file=$((current_file + 1))
    echo "Processing file $current_file of $total_files: $input_file"

    # Determine the output file path
    filename=$(basename "$input_file")
    filename_no_ext="${filename%.*}"
    extension="${filename##*.}"
    output_subdir=$(dirname "$input_file" | sed "s#^$START_DIR##")
    output_dir_full="$OUTPUT_DIR$output_subdir/audio"
    mkdir -p "$output_dir_full"
    output_file="$output_dir_full/${filename_no_ext}.opus"

    # Skip if output file already exists (resume functionality)
    if [ -f "$output_file" ]; then
        echo "Output file already exists. Skipping."
        continue
    fi

    # Run FFmpeg conversion and redirect output to log file
    eval $FFMPEG_CMD >> "$LOG_FILE" 2>&1

    # Calculate elapsed time and estimate remaining time
    elapsed_time=$(( $(date +%s) - start_time ))
    avg_time_per_file=$(echo "$elapsed_time / $current_file" | bc -l)
    remaining_files=$(( total_files - current_file ))
    estimated_remaining_time=$(echo "$avg_time_per_file * $remaining_files" | bc -l)

    # Convert estimated_remaining_time to integer
    estimated_remaining_time_int=$(printf "%.0f" "$estimated_remaining_time")

    # Display estimated time remaining
    printf "Estimated time remaining: %02dh:%02dm:%02ds\n" \
        $((estimated_remaining_time_int/3600)) \
        $(( (estimated_remaining_time_int%3600)/60 )) \
        $((estimated_remaining_time_int%60))
done

echo "Conversion complete."
