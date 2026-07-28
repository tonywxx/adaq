#include <math.h>
#include <stdio.h>
#include <string.h>

#include "ta_abstract.h"
#include "ta_func.h"

#define LENGTH 512

static double open_[LENGTH], high_[LENGTH], low_[LENGTH], close_[LENGTH], volume_[LENGTH];
static int first = 1;

static void print_number(double value) {
    if (isfinite(value)) printf("%.17g", value); else printf("null");
}

static void emit(const TA_FuncInfo *info, void *opaque) {
    (void)opaque;
    if (strcmp(info->name, "MAVP") == 0) return;
    TA_ParamHolder *holder = NULL;
    if (TA_ParamHolderAlloc(info->handle, &holder) != TA_SUCCESS) return;
    for (unsigned int index = 0; index < info->nbInput; index++) {
        const TA_InputParameterInfo *input;
        TA_GetInputParameterInfo(info->handle, index, &input);
        if (input->type == TA_Input_Price) {
            TA_SetInputParamPricePtr(holder, index, open_, high_, low_, close_, volume_, NULL);
        } else if (input->type == TA_Input_Real) {
            TA_SetInputParamRealPtr(holder, index, close_);
        } else {
            TA_ParamHolderFree(holder);
            return;
        }
    }
    double real[10][LENGTH] = {{0}};
    int integer[10][LENGTH] = {{0}};
    for (unsigned int index = 0; index < info->nbOutput; index++) {
        const TA_OutputParameterInfo *output;
        TA_GetOutputParameterInfo(info->handle, index, &output);
        if (output->type == TA_Output_Real) TA_SetOutputParamRealPtr(holder, index, real[index]);
        else TA_SetOutputParamIntegerPtr(holder, index, integer[index]);
    }
    int begin = 0, count = 0;
    if (TA_CallFunc(holder, 0, LENGTH - 1, &begin, &count) != TA_SUCCESS) {
        TA_ParamHolderFree(holder);
        return;
    }
    if (!first) printf(",\n");
    first = 0;
    printf("    {\"rawName\":\"%s\",\"outputs\":[", info->name);
    for (unsigned int index = 0; index < info->nbOutput; index++) {
        const TA_OutputParameterInfo *output;
        TA_GetOutputParameterInfo(info->handle, index, &output);
        if (index) printf(",");
        printf("{\"rawName\":\"%s\",\"begin\":%d,\"values\":[", output->paramName, begin);
        for (int offset = 0; offset < count; offset++) {
            if (offset) printf(",");
            if (output->type == TA_Output_Real) print_number(real[index][offset]);
            else printf("%d", integer[index][offset]);
        }
        printf("]");
        printf("}");
    }
    printf("]}");
    TA_ParamHolderFree(holder);
}

int main(void) {
    for (int index = 0; index < LENGTH; index++) {
        close_[index] = 0.1 + (index % 50) / 100.0;
        open_[index] = close_[index] - 0.01;
        high_[index] = close_[index] + 0.01;
        low_[index] = close_[index] - 0.02;
        volume_[index] = 10.0 + index;
    }
    if (TA_Initialize() != TA_SUCCESS) return 1;
    TA_SetUnstablePeriod(TA_FUNC_UNST_ALL, 0);
    TA_SetCompatibility(TA_COMPATIBILITY_DEFAULT);
    TA_RestoreCandleDefaultSettings((TA_CandleSettingType)11);
    printf("{\n  \"source\": \"TA-Lib C abstract API v0.7.1\",\n  \"length\": %d,\n  \"indicators\": [\n", LENGTH);
    TA_ForEachFunc(emit, NULL);
    printf("\n  ]\n}\n");
    return 0;
}
