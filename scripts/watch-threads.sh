#!/bin/sh

watch -n 0.5 ps H -o euser,pid,tid,comm,%mem,rss,%cpu,psr -p `pgrep aquatic` 
