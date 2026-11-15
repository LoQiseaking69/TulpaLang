# Linux single-board computer (Raspberry Pi, ...): a native executable.
#   make            ->  ./@NAME@     (run with sudo, or add your user to gpio/i2c/spi groups)
#   make sim        ->  ./@NAME@-sim (same program on the simulated bus, any computer)
TULPA ?= tulpa
@NAME@: app.rav
	$(TULPA) build --target linux --exe app.rav -o $@
sim: app.rav
	$(TULPA) build --target sim --exe app.rav -o @NAME@-sim
clean:
	rm -f @NAME@ @NAME@-sim *.o *.ll *.h
.PHONY: sim clean
