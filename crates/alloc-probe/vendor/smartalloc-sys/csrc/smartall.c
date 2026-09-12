/*

			 S M A R T A L L O C
			Smart Memory Allocator

	Evolved   over	 several  years,  starting  with  the  initial
	SMARTALLOC code for AutoSketch in 1986, guided	by  the  Blind
	Watchbreaker,  John  Walker.  Isolated in this general-purpose
	form in  September  of	1989.	Updated  with  be  more  POSIX
	compliant  and	to  include Web-friendly HTML documentation in
	October  of  1998  by  the  same  culprit.    For   additional
	information and the current version visit the Web page:

		  http://www.fourmilab.ch/smartall/

*/

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>

/*LINTLIBRARY*/

#ifdef SMARTALLOC

typedef enum
{
	False = 0,
	True = 1
} Boolean;

#define EOS '\0' /* End of string sentinel */
#define V (void)
#define sm_min(a, b) ((a) < (b) ? (a) : (b))

/*  Queue data structures  */

/*  General purpose queue  */

struct queue
{
	struct queue *qnext, /* Next item in queue */
		*qprev;			 /* Previous item in queue */
};

/*  Memory allocation control structures and storage.  */

/*  SM_ALIGN  --  Alignment of the pointer handed back to the caller.

	Why this is patched in this locally vendored copy: `sizeof(struct abufhead)`
	is 40 bytes on x86-64 and `smalloc` returns `malloc base + sizeof(abufhead)`.
	malloc only guarantees 16-byte alignment, so the caller-visible pointer was
	**8-byte aligned** -- which violates Rust's `GlobalAlloc` contract (the pointer
	must satisfy `layout.align()`).  Used as a global allocator the symptom is a
	SIGSEGV in the first 16-byte SIMD load (`movdqa`), e.g. inside hashbrown's
	table resize, before a single test runs.

	Fix: over-allocate by SM_ALIGN, lift the block base up to a SM_ALIGN boundary,
	keep the header immediately below the caller pointer (so `sm_free` still finds
	it at `ptr - sizeof(abufhead)`) and remember the real malloc base in the header,
	because the lifted base is not what `free()` must be given.
	`sizeof(struct abufhead)` is padded to a multiple of SM_ALIGN so that
	`base + sizeof(abufhead)` is SM_ALIGN-aligned. */

#define SM_ALIGN 64

struct abufhead
{
	struct queue abq;		 /* Links on allocated queue */
	unsigned ablen;			 /* Buffer length in bytes */
	char *abfname;			 /* File name pointer */
	unsigned short ablineno; /* Line number of allocation */
	void *abraw;			 /* Real malloc() base (the lifted base is not) */
	char abpad[16];			 /* Pad sizeof(struct abufhead) to 64 = SM_ALIGN */
};

static struct queue abqueue = {/* Allocated buffer queue */
							   &abqueue, &abqueue};

static Boolean bufimode = False; /* Buffers not tracked when True */

/*  Queue functions  */

static void qinsert();
static struct queue *qdchain();

/*  SMALLOC  --  Allocate buffer, enqueing on the orphaned buffer
		 tracking list.  */

static void *smalloc(fname, lineno, nbytes)
char *fname;
int lineno;
unsigned nbytes;
{
	char *buf, *raw, *base;
	unsigned total;
	struct abufhead *h;

	/* Note:  Unix	MALLOC	actually  permits  a zero length to be
	   passed and allocates a valid block with  zero  user	bytes.
	   Such  a  block  can	later  be expanded with realloc().  We
		   disallow this based on the belief that it's better to  make
	   a  special case and allocate one byte in the rare case this
	   is desired than to miss all the erroneous occurrences where
	   buffer length calculation code results in a zero.  */

	assert(nbytes > 0);

	/* Layout: [malloc base .. pad .. lifted base][abufhead][user bytes][sentinel].
	   `total` covers the worst-case pad (the base moves up by at most SM_ALIGN-1). */
	total = nbytes + (unsigned)sizeof(struct abufhead) + 1 + SM_ALIGN;
	if ((raw = malloc(total)) == NULL)
		return NULL;

	base = (char *)(((unsigned long)raw + (SM_ALIGN - 1)) &
					~(unsigned long)(SM_ALIGN - 1));
	h = (struct abufhead *)base;
	h->abraw = raw;
	h->ablen = (unsigned)(sizeof(struct abufhead) + nbytes + 1);
	h->abfname = bufimode ? NULL : fname;
	h->ablineno = (unsigned short)lineno;
	/* Enqueue buffer on allocated list */
	qinsert(&abqueue, (struct queue *)base);
	/* Emplace end-clobber detector at end of buffer */
	base[h->ablen - 1] = (char)((((long)base) & 0xFF) ^ 0xC5);

	buf = base + sizeof(struct abufhead); /* User data start: SM_ALIGN-aligned */
	return (void *)buf;
}

/*  SM_FREE  --  Update free pool availability.  FREE is never called
		 except  through  this interface or by actuallyfree().
		 free(x)  is  defined  to  generate  a	call  to  this
		 routine.  */

void sm_free(fp) void *fp;
{
	char *cp = (char *)fp;
	char *raw;
	struct queue *qp;

	assert(cp != NULL); /* Better not release a null buffer, guy! */

	cp -= sizeof(struct abufhead);
	qp = (struct queue *)cp;
	/* Read the malloc base out *before* the wipe below (it lives in the header). */
	raw = ((struct abufhead *)cp)->abraw;

	/* The following assertions will catch virtually every release
		   of an address which isn't an allocated buffer. */

	assert(qp->qnext->qprev == qp); /* Validate queue links */
	assert(qp->qprev->qnext == qp);

	/* The following assertion detects storing off the  end  of  the
	   allocated  space in the buffer by comparing the end of buffer
	   checksum with the address of the buffer.  */

	assert(((unsigned char *)cp)[((struct abufhead *)cp)->ablen - 1] ==
		   ((((long)cp) & 0xFF) ^ 0xC5));

	V qdchain(qp);

	/* Now we wipe the contents of	the  just-released  buffer  with
		   "designer  garbage"  (Duff  Kurland's  phrase) of alternating
	   bits.  This is intended to ruin the day for any miscreant who
		   attempts to access data through a pointer into storage that's
	   been previously released. */

	V memset(cp, 0xAA, (int)((struct abufhead *)cp)->ablen);

	free(raw);
}

/*  SM_MALLOC  --  Allocate buffer.  NULL is returned if no memory
		   was available.  */

void *sm_malloc(fname, lineno, nbytes)
char *fname;
int lineno;
unsigned nbytes;
{
	void *buf;

	if ((buf = smalloc(fname, lineno, nbytes)) != NULL)
	{

		/* To catch sloppy code that assumes  buffers  obtained  from
		   malloc()	are  zeroed,  we  preset  the buffer contents to
			   "designer garbage" consisting of alternating bits.  */

		V memset(buf, 0x55, (int)nbytes);
	}
	return buf;
}

/*  SM_CALLOC  --  Allocate an array and clear it to zero.  */

void *sm_calloc(fname, lineno, nelem, elsize)
char *fname;
int lineno;
unsigned nelem, elsize;
{
	void *buf;

	if ((buf = smalloc(fname, lineno, nelem * elsize)) != NULL)
	{
		V memset(buf, 0, (int)(nelem * elsize));
	}
	return buf;
}

/*  SM_REALLOC	--  Adjust the size of a  previously  allocated  buffer.
					Note  that  the trick of "resurrecting" a previously
			freed buffer with realloc() is NOT supported by this
			function.	Further, because of the need to maintain
			our control storage, SM_REALLOC must always allocate
			a  new  block  and	copy  the data in the old block.
			This may result in programs which make heavy use  of
			realloc() running much slower than normally.  */

void *sm_realloc(fname, lineno, ptr, size)
char *fname;
int lineno;
void *ptr;
unsigned size;
{
	unsigned osize;
	void *buf;

	assert(size > 0);

	/*  If	the  old  block  pointer  is  NULL, treat realloc() as a
	   malloc().  SVID is silent  on  this,  but  many  C  libraries
	   permit this.  */

	if (ptr == NULL)
		return sm_malloc(fname, lineno, size);

	/* If the old and new sizes are the same, be a nice guy and just
	   return the buffer passed in.  */

	osize = ((struct abufhead *)(((char *)ptr) -
								 sizeof(struct abufhead)))
				->ablen -
			(sizeof(struct abufhead) + 1);
	if (size == osize)
	{
		return ptr;
	}

	/* Sizes differ.  Allocate a new buffer of the	requested  size.
		   If  we  can't  obtain  such a buffer, act as defined in SVID:
	   return NULL from  realloc()	and  leave  the  buffer  in  PTR
	   intact.  */

	if ((buf = smalloc(fname, lineno, size)) != NULL)
	{
		V memcpy(buf, ptr, (int)sm_min(size, osize));
		/* If the new buffer is larger than the old, fill the balance
			   of it with "designer garbage". */
		if (size > osize)
		{
			V memset(((char *)buf) + osize, 0x55, (int)(size - osize));
		}

		/* All done.  Free and dechain the original buffer. */

		sm_free(ptr);
	}
	return buf;
}

/*  ACTUALLYMALLOC  --	Call the system malloc() function to obtain
			storage which will eventually be released
			by system or library routines not compiled
			using SMARTALLOC.  */

void *actuallymalloc(size)
unsigned size;
{
	return malloc(size);
}

/*  ACTUALLYCALLOC  --	Call the system calloc() function to obtain
			storage which will eventually be released
			by system or library routines not compiled
			using SMARTALLOC.  */

void *actuallycalloc(nelem, elsize)
unsigned nelem, elsize;
{
	return calloc(nelem, elsize);
}

/*  ACTUALLYREALLOC  --  Call the system realloc() function to obtain
			 storage which will eventually be released
			 by system or library routines not compiled
			 using SMARTALLOC.  */

void *actuallyrealloc(ptr, size) void *ptr;
unsigned size;
{
	return realloc(ptr, size);
}

/*  ACTUALLYFREE  --  Interface to system free() function to release
			  buffers allocated by low-level routines. */

void actuallyfree(cp) void *cp;
{
	free(cp);
}

/*  SM_DUMP  --  Print orphaned buffers (and dump them if BUFDUMP is
		 True). */

void sm_dump(bufdump)
	Boolean bufdump;
{
	struct abufhead *ap = (struct abufhead *)abqueue.qnext;

	while (ap != (struct abufhead *)&abqueue)
	{

		if ((ap == NULL) ||
			(ap->abq.qnext->qprev != (struct queue *)ap) ||
			(ap->abq.qprev->qnext != (struct queue *)ap))
		{
			V fprintf(stderr,
					  "\nOrphaned buffers exist.  Dump terminated following\n");
			V fprintf(stderr,
					  "  discovery of bad links in chain of orphaned buffers.\n");
			V fprintf(stderr,
					  "  Buffer address with bad links: %lx\n", (long)ap);
			break;
		}

		if (ap->abfname != NULL)
		{
			unsigned memsize = ap->ablen - (sizeof(struct abufhead) + 1);
			char errmsg[80];

			V sprintf(errmsg,
					  "Orphaned buffer:  %6u bytes allocated at line %d of %s\n",
					  memsize, ap->ablineno, ap->abfname);
			V fprintf(stderr, "%s", errmsg);
			if (bufdump)
			{
				unsigned llen = 0;
				char *cp = ((char *)ap) + sizeof(struct abufhead);

				errmsg[0] = EOS;
				while (memsize)
				{
					if (llen >= 16)
					{
						V strcat(errmsg, "\n");
						llen = 0;
						V fprintf(stderr, "%s", errmsg);
						errmsg[0] = EOS;
					}
					V sprintf(errmsg + strlen(errmsg), " %02X",
							  (*cp++) & 0xFF);
					llen++;
					memsize--;
				}
				V fprintf(stderr, "%s\n", errmsg);
			}
		}
		ap = (struct abufhead *)ap->abq.qnext;
	}
}

/*  SM_STATIC  --  Orphaned buffer detection can be disabled  (for  such
		   items  as buffers allocated during initialisation) by
		   calling   sm_static(1).    Normal   orphaned   buffer
		   detection  can be re-enabled with sm_static(0).  Note
		   that all the other safeguards still apply to  buffers
		   allocated  when  sm_static(1)  mode is in effect.  */

void sm_static(mode) int mode;
{
	bufimode = (Boolean)(mode != 0);
}

/*  Queue manipulation functions.  */

/*  QINSERT  --  Insert object at end of queue	*/

static void qinsert(qhead, object) struct queue *qhead, *object;
{
	assert(qhead->qprev->qnext == qhead);
	assert(qhead->qnext->qprev == qhead);

	object->qnext = qhead;
	object->qprev = qhead->qprev;
	qhead->qprev = object;
	object->qprev->qnext = object;
}

/*  QREMOVE  --  Remove object from queue.  Returns NULL if queue empty  */

static struct queue *qremove(qhead)
struct queue *qhead;
{
	struct queue *object;

	assert(qhead->qprev->qnext == qhead);
	assert(qhead->qnext->qprev == qhead);

	if ((object = qhead->qnext) == qhead)
		return NULL;
	qhead->qnext = object->qnext;
	object->qnext->qprev = qhead;
	return object;
}

/*  QDCHAIN  --  Dequeue an item from the middle of a queue.  Passed
		 the queue item, returns the (now dechained) queue item. */

static struct queue *qdchain(qitem)
struct queue *qitem;
{
	assert(qitem->qprev->qnext == qitem);
	assert(qitem->qnext->qprev == qitem);

	return qremove(qitem->qprev);
}
#endif
