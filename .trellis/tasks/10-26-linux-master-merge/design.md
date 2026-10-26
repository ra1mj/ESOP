# Design

Retain the existing Linux R0-R2 architecture and approved public interfaces.
Use focused independent reviews and executable reproduction to choose fixes;
do not add capabilities merely to increase the change count. Keep protocol,
simulation and lifecycle authorities independent. Each worker owns a distinct
review surface; coordinate confirmed fixes before modifying shared files.

Integrate using the existing feature branch and normal Git merge/fast-forward
rules after fetching origin and inspecting branch protection. Preserve history,
do not force push, and verify the resulting remote main identity and CI status.
