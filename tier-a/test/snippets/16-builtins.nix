builtins.map (x: x * 2) (builtins.filter (x: x > 1) [ 1 2 3 ])
