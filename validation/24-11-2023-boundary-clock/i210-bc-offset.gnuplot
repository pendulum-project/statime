set yrange [-6000:3000]
set xrange [0:7200]
set xlabel "time (s)"
set ylabel "offset (ns)"
plot 'i210-bc-offset.dat' using ($1*10) pt 7 black title "Offset to GM"
