{
  inputs.dry-flakes.url = "github:nejucomo/dry-flakes";

  outputs = inputs: inputs.dry-flakes.lib.mkFlakeOutputs { cargoWorkspace = ./.; };
}
