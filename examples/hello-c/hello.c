#include <stdio.h>
#include <sys/utsname.h>

int
main(void)
{
	struct utsname u;

	if (uname(&u) == -1) {
		perror("uname");
		return 1;
	}
	printf("hello from %s %s on %s\n", u.sysname, u.release, u.machine);
	return 0;
}
