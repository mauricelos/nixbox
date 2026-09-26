let a = 1; src = { b = 2; c = 3; }; in { inherit a; inherit (src) b c; }
