#!/bin/sh

watch -d -n 0.5 ps H -o euser,pid,tid,comm,%mem,rss,%cpu,psr -p `pgrep aquatic` 
