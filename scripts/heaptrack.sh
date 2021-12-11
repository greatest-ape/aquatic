#!/bin/bash

heaptrack --pid $(pgrep "^aquatic_[a-z]{1,4}$")
