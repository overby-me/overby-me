#include <stdio.h>
#include "greet.h"
#include "version.h"
int main(void) { printf("%s v%s\n", greeting(), APP_VERSION); return 0; }
