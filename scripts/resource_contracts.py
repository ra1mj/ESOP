"""Shared bounded SDO resource evidence for generated and measured reports."""

SDO_RESOURCE_FIELDS = {
    "runtime_sdo_requests", "max_sdo_object_bytes", "sdo_object_storage_bytes"
}


def validate_sdo_resources(resources: dict, path: str) -> None:
    present = SDO_RESOURCE_FIELDS.intersection(resources)
    if not present:
        return  # Backward-compatible esop.product-build-input.v1 baseline.
    if present != SDO_RESOURCE_FIELDS:
        raise ValueError(f"{path} requires all three SDO capacity fields together")
    requests = resources["runtime_sdo_requests"]
    size = resources["max_sdo_object_bytes"]
    storage = resources["sdo_object_storage_bytes"]
    if type(requests) is not int or not 1 <= requests <= 64:
        raise ValueError(f"{path}.runtime_sdo_requests must be 1..64")
    if type(size) is not int or not 1 <= size <= 65536:
        raise ValueError(f"{path}.max_sdo_object_bytes must be 1..65536")
    if type(storage) is not int or storage != requests * size:
        raise ValueError(f"{path}.sdo_object_storage_bytes must equal requests * object bytes")
