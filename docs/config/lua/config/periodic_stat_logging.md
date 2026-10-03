---
tags:
  - debugging
---
# `periodic_stat_logging = 0`

If non-zero, specifies the period (in seconds) at which various statistics are
logged. There is a minimum period of 10 seconds.

设为 0（默认）时不安装统计记录器，热重载把它从 0 改为非 0 需要重启 GUI
才生效；非 0 之间的改动热重载即生效。
