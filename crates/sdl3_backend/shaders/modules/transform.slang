#language slang 2026

module transform;

public namespace color {
    [ForceInline]
    public func linear_to_srgb(color: float3) -> float3 {
        return float3(
            linear_to_srgb_component(color.r),
            linear_to_srgb_component(color.g),
            linear_to_srgb_component(color.b)
        );
    }

    [ForceInline]
    func linear_to_srgb_component(value: float) -> float {
        let is_linear = value < 0.0031308;
        let linear_result = value * 12.92;
        let power_result = 1.055 * pow(abs(value), 1.0 / 2.4) - 0.055;
        return is_linear ? linear_result : power_result;
    }

    [ForceInline]
    public func srgb_to_linear(color: float3) -> float3 {
        return float3(
            srgb_to_linear_component(color.r),
            srgb_to_linear_component(color.g),
            srgb_to_linear_component(color.b)
        );
    }

    [ForceInline]
    func srgb_to_linear_component(value: float) -> float {
        let is_linear = value < 0.04045;
        let linear_result = value / 12.92;
        let power_result = pow(abs(value + 0.055) / 1.055, 2.4);
        return is_linear ? linear_result : power_result;
    }
}
