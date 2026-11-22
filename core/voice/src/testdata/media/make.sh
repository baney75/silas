#!/bin/sh
# Makes the decoder's test files: one second of a 440 Hz tone at half scale, stereo at 22.05 kHz
# (Opus is 48 kHz mono), in each format Silas reads, plus a video with no sound.
set -e
cd "$(dirname "$0")"
TONE="-f lavfi -i aevalsrc=0.5*sin(440*2*PI*t)|0.5*sin(440*2*PI*t):s=22050:d=1"
VIDEO="-f lavfi -i color=c=black:s=32x32:d=1:r=5"
q="-loglevel error -y"
o="-map_metadata -1 -fflags +bitexact"
ffmpeg $q $TONE $o tone.wav
ffmpeg $q $TONE -c:a libmp3lame -b:a 32k $o tone.mp3
ffmpeg $q $TONE -c:a aac -b:a 32k $o tone.m4a
ffmpeg $q $TONE -c:a alac $o alac.m4a
ffmpeg $q $TONE $o tone.flac
ffmpeg $q $TONE -c:a libvorbis -q:a 0 $o tone.ogg
ffmpeg $q -f lavfi -i "aevalsrc=0.5*sin(440*2*PI*t):s=48000:d=1" -c:a libopus -b:a 16k $o tone.opus
ffmpeg $q $VIDEO $TONE -c:v libx264 -c:a aac -b:a 32k -shortest $o tone.mp4
ffmpeg $q $VIDEO $TONE -c:v libx264 -c:a aac -b:a 32k -shortest $o tone.mov
ffmpeg $q $VIDEO $TONE -c:v libvpx-vp9 -c:a libopus -b:a 16k -shortest $o tone.webm
ffmpeg $q $VIDEO $TONE -c:v libvpx -c:a libvorbis -q:a 0 -shortest $o vorbis.webm
ffmpeg $q $VIDEO -c:v libx264 $o silent-video.mp4
