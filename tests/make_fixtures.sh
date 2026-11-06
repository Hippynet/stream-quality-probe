#!/bin/sh
# Generate synthetic test streams with known faults into tests/fixtures/.
# faults.mp3 (93 s): stereo 15s | dual-mono 15s | out-of-phase 15s | silence 6s |
#                    left-only 15s | clipping 12s | stereo 15s
set -e
cd "$(dirname "$0")"
mkdir -p fixtures && cd fixtures
STEREO='aevalsrc=0.25*sin(2*PI*440*t)+0.12*(2*random(0)-1)|0.25*sin(2*PI*660*t)+0.12*(2*random(1)-1):s=44100'
MONO='aevalsrc=0.25*sin(2*PI*440*t)*(0.6+0.4*sin(2*PI*0.5*t))+0.12*(2*random(0)-1):s=44100:d=15'
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "$STEREO:d=15" -f lavfi -i "$MONO" -f lavfi -i "$MONO" \
  -f lavfi -i "anullsrc=r=44100:cl=stereo:d=6" -f lavfi -i "$MONO" \
  -f lavfi -i "aevalsrc=0.9*sin(2*PI*330*t)+0.3*(2*random(0)-1)|0.9*sin(2*PI*495*t)+0.3*(2*random(1)-1):s=44100:d=12" \
  -f lavfi -i "$STEREO:d=15" \
  -filter_complex "[1]pan=stereo|c0=c0|c1=c0[m];[2]pan=stereo|c0=c0|c1=-1*c0[p];[4]pan=stereo|c0=c0|c1=0*c0[l];[5]volume=2.5[c];[0][m][p][3][l][c][6]concat=n=7:v=0:a=1,aformat=sample_fmts=s16[out]" \
  -map "[out]" -c:a libmp3lame -b:a 128k faults.mp3
# 24 kb/s source re-encoded at 192 kb/s (~4 kHz bandwidth)
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "$STEREO:d=60" -c:a libmp3lame -b:a 24k -ar 22050 low.mp3
ffmpeg -hide_banner -loglevel error -y -i low.mp3 -ar 44100 -c:a libmp3lame -b:a 192k upscaled.mp3
# genuinely mono-encoded stream
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "aevalsrc=0.25*sin(2*PI*440*t)+0.12*(2*random(0)-1):s=44100:d=60" \
  -c:a libmp3lame -b:a 64k -ac 1 monostream.mp3
rm -f low.mp3
# live HLS source: 4 s AAC segments over two minutes (served as a sliding window)
rm -rf hls && mkdir hls
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "$STEREO:d=120" -c:a aac -b:a 96k \
  -f hls -hls_time 4 -hls_list_size 0 -hls_segment_filename hls/seg%03d.ts hls/full.m3u8
ls -la
