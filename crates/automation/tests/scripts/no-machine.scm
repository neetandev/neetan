;; Running before construction raises neetan/no-machine and escapes.
(import (scheme base) (neetan automation 1))
(run-frames! #f 1)
(execution-result 'OK)
