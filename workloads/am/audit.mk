# Included after each upstream kernel Makefile. No link or media conversion.
.PHONY: audit-objects
audit-objects: $(addprefix $(DST_DIR)/,$(addsuffix .o,$(basename $(filter %.c %.cc %.cpp,$(SRCS)))))
