#!/bin/bash

while true; do
    for speed in {0..120}; do
        # Write the current number to /data/speed.txt
        echo $speed > data/speed.txt
        echo "Speed: $speed"

        # Wait for 20 milliseconds
        
        sleep 0.02
    done
done
