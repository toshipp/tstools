all: tstools

tstools:
	go build .

clean:
	rm -f tstools

.PHONY: all clean
